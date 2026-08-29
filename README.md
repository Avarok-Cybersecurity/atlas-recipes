# Atlas recipes and `atlasctl`

Recipes for [Atlas](https://github.com/Avarok-Cybersecurity/atlas), the pure-Rust
LLM inference server for NVIDIA DGX Spark (GB10), plus **`atlasctl`**, the
launcher that runs them.

A recipe describes one model deployment — the checkpoint, the container image,
and the serve settings it was validated under. `atlasctl` reads a recipe and
runs the `docker run` it implies.

## Install

Linux and macOS:

```sh
curl -fsSL https://atlasinference.io/install.sh | sh
```

Windows, in PowerShell:

```powershell
irm https://atlasinference.io/install.ps1 | iex
```

Or, if you already have the toolchains:

```sh
cargo install atlasctl        # from crates.io
uvx pyatlasctl list            # from PyPI, no install step
```

The installer downloads a prebuilt binary, verifies its SHA-256 against the
release, and puts it in `~/.local/bin` (`%LOCALAPPDATA%\Programs\atlasctl` on
Windows). It needs no Python and no Rust toolchain. Running it again on a
machine that already has atlasctl is an upgrade, or — when the version is
already current — a way to start an agent that is installed but stopped.
`sh scripts/install.sh --uninstall` reverses it; on Windows,
`atlasctl agent uninstall` removes the task and the binary can be deleted from
the install directory above.

The background agent is a `systemd --user` service on Linux, a launchd
LaunchAgent on macOS, and a Task Scheduler task at logon on Windows — a task
rather than a service because a service runs in session 0, which cannot reach
Docker Desktop's per-user named pipe.

## Use

```sh
atlasctl list                              # what is available
atlasctl show qwen3.6-35b-a3b-fp8-mtp      # what a recipe does
atlasctl run qwen3.6-35b-a3b-fp8-mtp       # serve it
atlasctl run <recipe> --print              # print the command instead of running it
atlasctl logs <recipe> --follow
atlasctl stop <recipe>
atlasctl status
atlasctl doctor                            # check this machine for problems (exit 1 if any)
```

`--print` is worth knowing about: it shows the exact `docker run` that `run`
would execute, so you can read it before trusting it, or run it yourself.
Add `--portable` to keep `$(id -u)` and `$HOME` symbolic for pasting elsewhere.

Multi-node recipes need one invocation per node:

```sh
# on the head
atlasctl run <recipe> --rank 0 --world-size 2 --master-addr 10.10.10.1
# on the worker
atlasctl run <recipe> --rank 1 --world-size 2 --master-addr 10.10.10.1
```

A multi-node recipe refuses to launch on a single node rather than quietly
serving something smaller than the recipe describes.

## Ports

Two, and they fail independently, which is why they are worth telling apart.

| port | bound on | who talks to it |
|---|---|---|
| 34333 | loopback only | the website, on this machine |
| 34334 | all interfaces | other machines — pairing, joining, and cluster work |

34333 never leaves the machine, so nothing in a firewall applies to it.

**34334 has to be reachable between machines.** If it is blocked, or something
else is holding it, everything local keeps working — the website still finds
this machine and still offers to add another — and the failure appears on the
OTHER machine, as:

```
joining the fleet at 192.168.68.67:34334…
error: ... Connection refused
```

`atlasctl doctor` reports both, separately:

```
agent:    ok (listening on 127.0.0.1:34333)
peers:    ok (accepting on 34334)
```

A `peers:` line that is not listening means this machine cannot be joined,
however healthy the rest of the output looks. The agent retries that port, so
the usual cause is another `atlasctl agent` already running here — and the
usual fix is to use that one rather than start a second.

## Where recipes come from

**Recipes are compiled into `atlasctl`.** A fresh install performs no network
access to find a recipe, because there is nothing to fetch — the corpus a binary
ships with is the corpus it was built from. Updating recipes means updating
`atlasctl`.

You can add your own registry:

```sh
atlasctl registry add myteam https://github.com/myteam/recipes.git
atlasctl run @myteam/my-recipe
```

A remote registry supplies recipe **data** and nothing else. It cannot cause a
command to run on your machine:

- recipe fields that executed code in the previous launcher — `pre_exec`,
  `post_exec`, `post_commands`, `mods`, `builder` — are refused wherever they
  appear, including in recipes we ship ourselves;
- container isolation comes from one reviewed profile in `atlasctl`, never from
  a recipe, so `executor_config` is refused too;
- there is no "trusted registry" concept in `atlasctl` at all. The mechanism
  does not exist, so no configuration edit can enable it.

A recipe carrying refused keys still appears in `atlasctl list --all`, with the
reason. A recipe that vanishes is harder to reason about than one that explains
itself.

Registry names are resolved locally: `atlas` is reserved for the built-in
corpus, and a bare recipe name always resolves to a built-in recipe first, so a
remote cannot shadow a shipped recipe by choosing its name.

## Replacing sparkrun

`atlasctl` replaces the `sparkrun` launcher. If you have sparkrun installed,
run `atlasctl doctor` — and read [SECURITY.md](SECURITY.md), which explains why
this exists and what to check.

Serve commands are byte-identical to sparkrun's across the whole recipe corpus;
see [docs/PARITY.md](docs/PARITY.md) for the comparison and for the differences
that are deliberate.

## Contributing a recipe

Add a YAML file under `recipes/<family>/`. The filename stem is the recipe name.

```yaml
recipe_version: "2"
model: org/Model-Name
runtime: atlas
container: avarok/atlas-gb10:latest
max_nodes: 1

metadata:
  description: |
    What this deployment is, and what it was measured at.
  maintainer: you

defaults:
  port: 8888
  max_model_len: 8192
  gpu_memory_utilization: 0.85
```

CI parses and renders every recipe in this repository, so a malformed recipe
fails the pull request rather than someone's machine. Please say in the
description what the settings were validated against — the numbers in these
files are the reason to trust them.

### A note on checkpoints

Recipes reference upstream HuggingFace repos by name, and **upstream can re-quantize a repo
in place**. That happened on 2026-07-10: `unsloth/Qwen3.6-{27B,35B-A3B}-NVFP4` were re-uploaded
in a mixed-precision NVFP4/FP8 layout, which no Atlas release could load — every user who
downloaded fresh hit `Weight '...weight_global_scale' not found in store`, while it kept
working for anyone with the old snapshot still cached.

The default 27B/35B NVFP4 recipes therefore now track the **`nvidia/*`** checkpoints,
whose on-disk format has been stable since 2026-05-29. Those are verified end-to-end on a
GB10 and are what you should use.

`-unsloth` recipes now exist, but only where a gate is actually measured on one — they
are deliberately not the defaults. Loading the mixed-precision layout took two fixes:
atlas#300 (the layer weights) and atlas#301 (the FP8 `lm_head`, plus per-row weight scales
that were being fed to a 128×128 block-scaled kernel — in-bounds, so no crash, just
silently wrong logits). Both are on `main` and verified on a GB10:

| checkpoint | throughput | correctness |
|---|---|---|
| `unsloth/Qwen3.6-27B-NVFP4` | 14.0 tok/s | pass |
| `unsloth/Qwen3.6-35B-A3B-NVFP4` | 123.4 tok/s | pass |

The two shipped so far are `qwen3.6-27b-nvfp4-unsloth` (the BFCL gate config) and
`qwen3.8-27b-nvfp4-unsloth` (the agentic gate config). Both pin an image that can load the
layout; on anything older they fail with `weight_global_scale not found`.

If a model suddenly fails to build with a missing `weight_global_scale` or a
`weight_scale` dtype error, you are almost certainly on a newer checkpoint than your Atlas
image — pull a newer `avarok/atlas-gb10:dev`.

## Catalogue

| Recipe | Model | Topology | Notes |
|---|---|---|---|
| `qwen3.6-35b-a3b-nvfp4` | nvidia/Qwen3.6-35B-A3B-NVFP4 | single | **DEFAULT 35B** — MTP K=1 (pinned; 116.5 tok/s), calibrated fp8 KV (128K), qwen3_coder agentic stack; requires :dev ≥ 2026-07-10 (atlas#287) |
| `qwen3.6-27b-nvfp4` | nvidia/Qwen3.6-27B-NVFP4 | single | **DEFAULT 27B** — dense hybrid SSM+Attn, MTP K=1 (pinned), bf16 KV, qwen3_coder agentic stack; requires :dev ≥ 2026-07-10 (atlas#287) |
| `qwen3.8-27b-nvfp4-unsloth` | unsloth/Qwen3.8-27B-NVFP4 | single | Dense hybrid SSM+Attn — the AGENTIC gate config: thinking ON, bf16 head + bf16 KV, 32K, MTP K=4, slai. Architecturally identical to Qwen3.6-27B (all 1968 tensors match); only the weights differ |
| `qwen3.6-35b-a3b-fp8-mtp` | Qwen/Qwen3.6-35B-A3B-FP8 | single | Flagship FP8 — native FP8, bf16 head + bf16 KV, 64K ctx, MTP K=2, live tool-call streaming |
| `qwen3.6-35b-a3b-fp8-bf16head` | Qwen/Qwen3.6-35B-A3B-FP8 | single | 32K safe profile of the FP8 flagship (same bf16 head/KV) |
| `qwen3.6-35b-a3b-fp8-nvfp4head` | Qwen/Qwen3.6-35B-A3B-FP8 | single | nvfp4 lm-head sibling — near-neutral wall, lower VRAM |
| `qwen3.6-27b-fp8-mtp` | Qwen/Qwen3.6-27B-FP8 | single | Dense hybrid SSM+Attn, **:dev + MTP K=1** → 15.6 tok/s (on :latest, or at K=2, it is 5.0), 60k ctx |
| `qwen3.5-35b-a3b-nvfp4` | Sehyo/Qwen3.5-35B-A3B-NVFP4 | single | MTP K=2, ~131 tok/s |
| `qwen3.5-27b-dense-nvfp4` | Kbenkhaled/Qwen3.5-27B-NVFP4 | single | Dense hybrid SSM+Attn, ~14 tok/s |
| `qwen3.5-122b-a10b-nvfp4-single` | Sehyo/Qwen3.5-122B-A10B-NVFP4 | single | Tight KV/seq budget, all 256 experts on one node |
| `qwen3.5-122b-a10b-nvfp4-ep2` | Sehyo/Qwen3.5-122B-A10B-NVFP4 | 2-node | EP=2 + MTP K=2 |
| `qwen3-next-80b-a3b-nvfp4` | nvidia/Qwen3-Next-80B-A3B-Instruct-NVFP4 | single | MTP, ~74-104 tok/s |
| `qwen3-coder-next-fp8` | Qwen/Qwen3-Coder-Next-FP8 | single | Native FP8, ~58 tok/s, BF16 KV |
| `qwen3-vl-30b-a3b-nvfp4` | ig1/Qwen3-VL-30B-A3B-Instruct-NVFP4 | single | Vision-language, ~97 tok/s |
| `minimax-m2.7-nvfp4-ep2` | lukealonso/MiniMax-M2.7-NVFP4 | 2-node | EP=2, BF16 KV bring-up, no MTP |
| `gemma-4-31b-nvfp4` | nvidia/Gemma-4-31B-IT-NVFP4 | single | Dense, sliding+full attention, gemma4 tool parser |
| `gemma-4-26b-a4b-nvfp4` | bg-digitalservices/Gemma-4-26B-A4B-it-NVFP4A16 | single | MoE GeGLU, ~67 tok/s |
| `nemotron-3-super-120b-a12b-nvfp4` | nvidia/NVIDIA-Nemotron-3-Super-120B-A12B-NVFP4 | single | LatentMoE, ~24 tok/s |
| `nemotron-3-nano-30b-a3b-nvfp4` | nvidia/NVIDIA-Nemotron-3-Nano-30B-A3B-NVFP4 | single | Mamba-2 + MoE, ~88 tok/s |
| `nemotron-3.5-lightning-30b-a3b-nvfp4` | nvidia/NVIDIA-Nemotron-3.5-Lightning-30B-A3B-NVFP4 | single | NoPE hybrid, 256K ctx, ~72 tok/s — needs atlas#487 + #462 |
| `mistral-small-4-119b-nvfp4` | mistralai/Mistral-Small-4-119B-2603-NVFP4 | single | MLA, BF16-only KV (mandatory) |

## Layout

```
recipes/
├── qwen3.5/
│   ├── qwen3.5-27b-dense-nvfp4.yaml
│   ├── qwen3.5-35b-a3b-nvfp4.yaml
│   ├── qwen3.5-122b-a10b-nvfp4-single.yaml
│   └── qwen3.5-122b-a10b-nvfp4-ep2.yaml
├── qwen3-next/qwen3-next-80b-a3b-nvfp4.yaml
├── qwen3-vl/qwen3-vl-30b-a3b-nvfp4.yaml
├── qwen3-coder-next/qwen3-coder-next-fp8.yaml
├── gemma4/{gemma-4-26b-a4b-nvfp4.yaml, gemma-4-31b-nvfp4.yaml}
├── nemotron-3-nano/nemotron-3-nano-30b-a3b-nvfp4.yaml
├── nemotron-3.5-lightning/nemotron-3.5-lightning-30b-a3b-nvfp4.yaml
├── nemotron-3-super/nemotron-3-super-120b-a12b-nvfp4.yaml
├── mistral-small-4/mistral-small-4-119b-nvfp4.yaml
└── minimax-m2.7/minimax-m2.7-nvfp4-ep2.yaml
```

atlasctl's recipe lookup is recursive within the `recipes` subtree, so the family-level grouping is purely cosmetic. Recipes are accessed by their file stem regardless of nesting.

## Hardware constraints captured in the recipes

Each recipe carries the production-validated KV/seq/MoE settings drawn from Atlas's `QUICKSTART.md`, the `scripts/sweep_all_models.sh` baseline, and the production `start-minimax-ep2.sh`/`start-ep2.sh` bring-up scripts. Notably:

- **Mistral Small 4** enforces `kv_cache_dtype: bf16` — FP8/NVFP4 KV destroys the MLA compressed latent (Atlas alpha-2.8 release announcement).
- **Qwen3-Coder-Next-FP8** requires `ssm_cache_slots: 0`, `oom_guard_mb: 1024`, and `kv_cache_dtype: bf16`.
- **122B EP=2** + **MiniMax M2.7 EP=2** carry matching `--speculative` / `--mtp-quantization` flags on both ranks (mismatched flags land MTP verify in the worker's SSM layer with no buffers allocated).
- **MiniMax M2.7 EP=2** is capped at `max_model_len: 12288` to fit the head's KV budget at `gpu_memory_utilization: 0.90` on the public `avarok/atlas-gb10:latest` image (live-validated 2026-05-08).

## Related

- Runtime: [`atlas` runtime in atlasctl](https://github.com/spark-arena/atlasctl) (PR #169)
- Engine: https://github.com/Avarok-Cybersecurity/atlas
- Docker image: [`avarok/atlas-gb10`](https://hub.docker.com/r/avarok/atlas-gb10)
- Discord: [Atlas-Inference](https://atlasinference.io)

## License

AGPL-3.0 — see [LICENSE](LICENSE). Matches the upstream [Atlas](https://github.com/Avarok-Cybersecurity/atlas) license.
