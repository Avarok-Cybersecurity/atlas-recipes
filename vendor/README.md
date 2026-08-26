# Vendored engine interfaces

## `serve-options.v1.json`

Every `spark serve` flag, reflected out of the engine's own clap definition by
`spark dump-serve-options` (see `crates/spark-server/src/cli/manifest.rs` in the
engine repository).

Regenerate deliberately:

```sh
spark dump-serve-options > vendor/serve-options.v1.json
cargo test -p atlasctl-core        # coverage check
```

**This is not a public format.** The engine deliberately refuses to derive
`Serialize` on `ServeArgs`, because a cross-repo wire format makes every rename
a compatibility break. A committed snapshot is the opposite of that promise: a
rename shows up here as a reviewable diff, and `flags::coverage` turns it into a
failing test. Nothing at runtime reads this file.

It answers three questions that cannot be recovered from reading a recipe:

- **Which flags exist.** Nine keys in shipping recipes were dropped on the floor
  for the life of this project because nothing could tell you they were real.
- **Which take a value.** `video_allow_ffmpeg: true` and `gdn_fused_norm: true`
  are written identically and emit differently.
- **What each accepts.** `scheduling_policy` offered `fcfs` for four releases;
  the engine takes only `fifo` and `slai`, so every launch that chose it died
  inside the container.

What it does *not* carry is ranges — clap has none — so every `Int`/`Float`
bound in `settings` is this project's own judgement. `every_shipped_recipe_value_satisfies_its_own_bound`
in `tests/golden.rs` is what keeps that judgement honest against the corpus.
