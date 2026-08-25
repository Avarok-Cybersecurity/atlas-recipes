# Parity with the reference implementation

atlasctl replaces the Python `sparkrun` launcher. The risk in that kind of
rewrite is not a loud failure — it is a *plausible* command that quietly
differs from the one every published benchmark was measured under. This
document records what was checked, what matches, and what deliberately does not.

## Serve commands: 28/28 byte-identical

Every launchable recipe in this repository was rendered by both implementations
and compared byte-for-byte:

- **atlasctl**: `translate()` under a fixed host snapshot, as frozen in
  `crates/atlasctl-core/tests/golden/`.
- **sparkrun 0.3.6**: `sparkrun run <recipe> --hosts localhost --dry-run`,
  taking the printed `Serve command:` line.

All 28 matched exactly — same flags, same values, same order. Multi-node
recipes were compared on their head rank, since sparkrun's dry run prints the
base command without per-rank coordination flags.

To reproduce:

```sh
cargo test -p atlasctl-core --test golden      # our side
sparkrun run recipes/<family>/<recipe>.yaml --hosts localhost --dry-run
```

## Deliberate divergences

These are intended and reviewed. Each is a behaviour change, not an accident.

### The container is launched in one phase, not two

sparkrun runs `docker run … sleep infinity` and then `docker exec`s the serve
command into it, so that `pre_exec` hooks have somewhere to run. atlasctl does
not support hooks, so it emits a single self-contained `docker run` that ends
in `spark serve …`.

Consequences, all of them improvements:

- the string we print is exactly what executes, so the copy button and the
  agent cannot disagree;
- `docker logs` shows serve output — under sparkrun PID 1 is `sleep`, so it
  does not;
- `--restart` restarts the model rather than a sleeping process;
- container exit equals serve exit, so status is truthful.

The cost: with `--rm`, a crashed serve removes its own logs. sparkrun's sleeping
container survived to be inspected.

### Recipe-supplied code is refused, not executed

`pre_exec`, `post_exec`, `post_commands`, `stop_after_post`, `mods`, `builder`
and `builder_config` are recognised and refused. `executor_config` is refused
too: container isolation comes from one reviewed profile, never from a recipe.

A recipe carrying any of them still *loads*, so `recipe list` and `recipe show`
can explain why it will not run. Two recipes in this repository are affected —
both `diffusion-gemma` files, which use `mods`.

### Only the `atlas` runtime launches

sparkrun infers a runtime for legacy recipes by inspecting their command
template. atlasctl does not guess what to execute; a recipe with no `runtime:`
is listed and explained rather than launched.

### Settings the flag table does not claim are reported

Both implementations emit only the flags in their table, so unclaimed recipe
settings do not reach `spark serve`. sparkrun drops them in silence; atlasctl
reports every one.

This is not hypothetical. Nine settings in this repository are affected today,
including `lm_head_dtype`, which appears in four recipes and is described in one
of them as a correctness pin. It has never reached the engine. Reconciling these
against real `spark serve` flags is tracked separately — this change only makes
the loss visible.

### Aliased settings collide instead of double-emitting

`max_num_batched_tokens` and `max_prefill_tokens` both render
`--max-prefill-tokens`. sparkrun emits the flag twice when both are set;
atlasctl rejects the config. No recipe here sets both.

## What is not yet covered

The comparison above is of *serve commands*. The docker wrapper differs by
design (single-phase, argv rather than `bash -c`), so it is not compared
mechanically. Multi-node behaviour beyond command rendering — rendezvous,
per-rank orchestration, fabric selection — is not yet implemented and therefore
not yet compared.
