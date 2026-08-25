# atlasctl

Launch [Atlas](https://github.com/Avarok-Cybersecurity/atlas) inference recipes
on NVIDIA DGX Spark (GB10) and other local accelerators.

A recipe describes one model deployment — the checkpoint, the container image,
and the serve settings it was validated under. `atlasctl` reads a recipe and
runs the `docker run` it implies.

```sh
uvx atlas-ctl list                              # what is available
uvx atlas-ctl show qwen3.6-35b-a3b-fp8-mtp      # what a recipe does
uvx atlas-ctl run qwen3.6-35b-a3b-fp8-mtp       # serve it
uvx atlas-ctl run <recipe> --print              # print the command instead
```

`uv tool install atlas-ctl` puts `atlasctl` on your PATH under its real name.

This wheel contains a self-contained Rust binary and no Python code. The
distribution is named `atlas-ctl` because `atlasctl` was already taken on PyPI
by an unrelated project.

**Recipes ship inside the binary.** A fresh install makes no network request to
resolve a recipe, and there is no "trusted registry" mechanism — a remote
registry can supply recipe data but can never cause a command to run. See
[SECURITY.md](https://github.com/Avarok-Cybersecurity/atlas-recipes/blob/main/SECURITY.md)
for why that matters and what it replaces.

Requires Docker to launch anything; `list`, `show`, and `run --print` work
without it. Linux and macOS; Windows is not supported.

AGPL-3.0-only.
