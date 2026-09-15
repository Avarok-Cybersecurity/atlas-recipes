# Bench jobs

A `bench`-granted peer can hand a node a commit and a certification gate;
the node builds that commit of its own Atlas checkout, runs the gate as a
child in its own process group, and hands the records back over the
pinned peer channel. `atlasctl bench` is the command that drives it;
`spark bench certify --with-nodes` in the Atlas repository drives
`atlasctl bench … --json`.

Everything below is the contract both sides pin in tests.

## What the grant permits

A `bench` grant (`atlasctl peer grant-bench <fingerprint>`) lets that peer:

* build **the checkout named in this node's `bench.yaml`** at any commit
  that is reachable from the configured `allowed_remote` (a commit the
  remote does not have is refused unless `allow_unpublished_shas` is set);
* run **one gate the built binary lists** (`spark benchmark list`), with
  parameters the same binary validates, under a fixed, scrubbed
  environment;
* read back the records that run wrote, and the child's log.

Nothing else. The request is a closed enum — commit, gate, parameters,
checkpoint name, box class, run cap, note — and the node renders the
command from its own configuration; there is no argv, path or URL in
the protocol for a filter to miss. The grant is separate from
`controller` and never implied by it. See `SECURITY.md`, "The bench
grant".

## What a node reports

`bench nodes` returns, per node: identity and agent version, whether bench
is on (and why not), the GPU (name, count, driver, CUDA, clock, temperature,
memory), **the live thermal facts** (`chassis_temps_c` from every sysfs
thermal zone, `throttle_thermal` from `nvidia-smi -q -d PERFORMANCE`,
`sm_clock_max_mhz`, `mem_total_kb`), alerts, the box class, the repo and its
remote, `ATLAS_HOME` and the signer fingerprint it would sign with, cached
builds, busy/queue state, disk and memory floors. Facts only: whether two
nodes are "the same box" for a speed-class gate is decided by the submitter
(Atlas, from these fields and again from the records), never by the node.

## `bench.yaml`

In the agent's config directory (`~/.config/atlasctl/` or `--config-dir`).
Absent → the bench surface is off and says so; present but invalid →
`agent run` exits 1 naming the key.

```yaml
atlas_repo: /workspace/atlas          # a git checkout; must have `allowed_remote`
atlas_home: /workspace/.atlas         # ATLAS_HOME for the child: signer + run history
hardware: gb10                        # the box class the records will name
cache_dir: /workspace/.atlasctl-bench # worktrees, builds, jobs, cargo target
allowed_remote: avarok                # a submitted commit must be reachable from here
env:
  PATH_PREPEND: /usr/local/cuda/bin   # prepended to PATH; every other key is exported as is
  CUDARC_CUDA_VERSION: "13000"
# Optional, with these defaults:
allow_unpublished_shas: false
queue_depth: 2
max_run_s: 10800
build_timeout_s: 3600
stall_timeout_s: 1800
cancel_grace_s: 30
min_free_fraction: 0.85
min_free_disk_bytes: 21474836480
keep_builds: 5
retain_jobs: 50
retain_days: 7
sync_recipes: true
collect_extra: []
serve_reuse: false
serve_release_after_s: 600
```

With `serve_reuse: true` the child runs as `spark benchmark run …
--serve-reuse --serve-lease-owner <agent pid>`: it does not load the
checkpoint in its own process but takes the server the previous job left
running — named in `<atlas_home>/serve-lease.json` — when that server is
provably the one it would have started (same binary bytes, same recipe
rendering with the same overrides, verified by Atlas over `GET
/serve-config`), replaces it otherwise, and leaves it up. Consecutive jobs
on one recipe pay for one model load. The agent treats that server as its
own tenant (its `spark` process, its GPU app and the memory it holds do
not make the box "busy"), stops it after `serve_release_after_s` with
nothing queued or running, and never touches a lease another owner wrote.

The child's environment is `HOME USER LANG TERM` from the agent, `PATH`
(with `PATH_PREPEND` in front), `ATLAS_HOME`, `CARGO_TARGET_DIR`
(`<cache_dir>/target`, shared across builds), and the `env` keys.
Nothing else leaks in.

## On disk

```
<cache_dir>/
  jobs/<job id>/job.json          the record: spec, state, pid, outcome
  jobs/<job id>/events.ndjson     the journal, one event per line, seq-numbered
  jobs/<job id>/child.log         the gate's combined stdout/stderr
  jobs/<job id>/artifacts/        what the run wrote, by name
  jobs/by-key/<job key>           idempotency index → job id
  build/<sha>/spark               the built binary
  build/<sha>/provenance.json     sha, binary sha256, built_at, bytes
  worktrees/<sha>/                the checkout at that commit
  target/                         cargo's target dir, shared
```

A cached build is reused **only** when `provenance.json` names the
requested sha **and** the binary on disk still hashes to
`provenance.binary_sha256`. Anything else — no provenance, another sha, a
tampered or truncated binary — is rebuilt. `spark` embeds no git sha, so
provenance is the only witness.

## A job's life

`Queued → Preparing → Building → Running → Collecting → Done`. Every
transition and every event is written before the next step, so a restart
finds the truth: a child that outlived the agent is resumed by its pid
and `/proc` start ticks; one that cannot be is recorded `Orphaned`.

The worker runs one job at a time and starts none while the box is busy:
a `spark` process (ours or not — except the server this agent's last job
left leased, see `serve_reuse`), a GPU compute app, host memory below
`min_free_fraction`, or the cache disk below `min_free_disk_bytes`.
Submission checks only the disk floor and the queue; the rest is checked
right before the child starts, because it changes.

Cancel is `SIGTERM` to the process group, `cancel_grace_s`, then
`SIGKILL`; idempotent; a job cancelled while queued is terminal at once.

## The event stream

```json
{"job":"jb-1757770000-1a2b3c4d","seq":7,"at_ms":1757770123000,"kind":"progress","phase":"isl 512 · conc 8 [3/8]","detail":""}
```

`seq` is 1-based and monotonic per job. Kinds: `queued`, `preparing`,
`build` (`cached`, `reason`), `built` (`binary_sha256`, `cached`, `secs`),
`running` (`pid`, `argv`), `progress`, `log` (`stream`, ≤ 64 `lines`),
`log_truncated`, `verdict` (`{kind: pass|fail|info, text}`), `artifact`
(`meta`), `done` (the outcome, flattened), and `heartbeat` every 10 s
(repeats `seq_high`; never journaled).

Outcomes: `completed` (`exit_code`, `verdict`, `record`, `signature`),
`failed` (`stage`, `reason`), `timed_out` (`stage`, `after_s`),
`cancelled` (`by`), `orphaned` (`reason`). **Only `completed` with a
`pass` verdict is a pass** — exit 0 without a record is a `collecting`
failure.

An attach from `from_seq` replays the journal from there and then
follows; a client that loses its link re-attaches from `last_seq + 1`
and misses nothing, duplicates nothing. Attaching past the end of a
finished job ends the stream on a terminal heartbeat.

## `atlasctl bench`

```
atlasctl bench nodes 10.10.10.2,dgx3.local          what each node can run
atlasctl bench submit  NODE --sha SHA --gate ID [--param k=v]… [--job-key KEY]
atlasctl bench attach  NODE JOB [--from-seq N] [--reconnect-for SECS]
atlasctl bench status  NODE [--job JOB]
atlasctl bench cancel  NODE JOB
atlasctl bench artifacts NODE JOB
atlasctl bench fetch   NODE JOB --out-dir DIR
atlasctl bench run     NODE --sha SHA --gate ID --out-dir DIR [--param k=v]…
```

Addresses: `ip[:port]`, `[v6]:port`, `host.local[:port]`,
`dns.name[:port]`; port omitted → 34334. A `.local` name the system
resolver cannot answer gets a three-second browse of atlasctl's own
service record and dials the port the node advertises.

`--json` puts exactly one document on stdout (one event per line for
`attach` and `run`, then a summary line for `run`) and nothing else there.
Errors under `--json` are one object:

```json
{"code":"refused:busy","message":"the box is busy (spark pid 3259905); retry after 60 s","node":"10.10.10.2","node_id":"…","fix":"retry after 60 s","retryable":true}
```

Exit codes:

| code | meaning |
|---|---|
| 0 | done — for `attach`/`run`, the job **passed** |
| 1 | bad arguments, or a local I/O failure |
| 2 | unreachable: the address did not answer, or the link broke |
| 3 | not paired, or paired but not granted `bench` |
| 4 | refused by the node (`refused:<code>`) |
| 5 | the job did not pass: failed, timed out, orphaned, or verdict not `pass` |
| 6 | the job was cancelled |
| 7 | the stream was lost and the re-attach budget ran out |
| 8 | the node's peer protocol is below bench |

`fetch` writes each artifact at its repo-relative path under `--out-dir`
(`.benchmarks/<gate>/<file>.json`, `.json.sig`, `.certify/<sha>/<gate>.log`),
verifies size and sha256 against what the node promised, refuses any
path that would land outside the directory, and never overwrites.

Submission is idempotent by `--job-key` (derived from sha, gate and
params when omitted): the same key returns the same job; the same key
with a different sha/gate/params is a `key_conflict`.
