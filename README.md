# Atlas Spark recipe registry

Official sparkrun recipes for [Atlas Spark](https://github.com/Avarok-Cybersecurity/atlas) — the pure-Rust LLM inference server for NVIDIA DGX Spark GB10. Recipes target the public `avarok/atlas-gb10:latest` Docker image and the `atlas` runtime in [sparkrun](https://github.com/spark-arena/sparkrun).

## Usage

The `@atlas` namespace is reserved in sparkrun and pre-registered in default installs, so no manual `sparkrun registry add` is needed:

```bash
sparkrun run @atlas/qwen3.5-35b-a3b-nvfp4
sparkrun run @atlas/minimax-m2.7-nvfp4-ep2          # 2-node EP=2
sparkrun run @atlas/qwen3.5-122b-a10b-nvfp4-ep2     # 2-node EP=2
```

If you're on an older sparkrun release that doesn't ship the reservation yet, add it manually:

```bash
sparkrun registry add https://github.com/Avarok-Cybersecurity/atlas-recipes.git
```

### Listing the recipes

The `atlas` registry ships **hidden** in sparkrun's defaults, so a bare `sparkrun list`
filters its recipes out — even though `sparkrun run @atlas/<recipe>` works, because `run`
resolves by qualified name and isn't visibility-gated. To see them:

```bash
sparkrun recipe list --registry atlas    # just the Atlas lineup
sparkrun list -a                         # everything, including hidden registries
sparkrun recipe show @atlas/qwen3.6-35b-a3b-nvfp4
```

`sparkrun search` has the same filter — pass `-a` or `--registry atlas`.

### A note on checkpoints

Recipes reference upstream HuggingFace repos by name, and **upstream can re-quantize a repo
in place**. That happened on 2026-07-10: `unsloth/Qwen3.6-{27B,35B-A3B}-NVFP4` were re-uploaded
in a mixed-precision NVFP4/FP8 layout, which no Atlas release could load — every user who
downloaded fresh hit `Weight '...weight_global_scale' not found in store`, while it kept
working for anyone with the old snapshot still cached.

The default 27B/35B NVFP4 recipes therefore now track the **`nvidia/*`** checkpoints,
whose on-disk format has been stable since 2026-05-29. Those are verified end-to-end on a
GB10 and are what you should use.

There is deliberately **no `-unsloth` recipe yet**, though the engine now supports those
checkpoints. Loading the mixed-precision layout took two fixes: atlas#300 (the layer
weights) and atlas#301 (the FP8 `lm_head`, plus per-row weight scales that were being fed
to a 128×128 block-scaled kernel — in-bounds, so no crash, just silently wrong logits).
Both are on `main` and verified on a GB10:

| checkpoint | throughput | correctness |
|---|---|---|
| `unsloth/Qwen3.6-27B-NVFP4` | 14.0 tok/s | pass |
| `unsloth/Qwen3.6-35B-A3B-NVFP4` | 123.4 tok/s | pass |

The recipes land once an `avarok/atlas-gb10:dev` image **carrying atlas#301** is published.
Until then a recipe pointing at those checkpoints would still fail on the image you'd
actually pull, and shipping a recipe that does not serve is worse than shipping none.

If a model suddenly fails to build with a missing `weight_global_scale` or a
`weight_scale` dtype error, you are almost certainly on a newer checkpoint than your Atlas
image — pull a newer `avarok/atlas-gb10:dev`.

## Catalogue

| Recipe | Model | Topology | Notes |
|---|---|---|---|
| `qwen3.6-35b-a3b-nvfp4` | nvidia/Qwen3.6-35B-A3B-NVFP4 | single | **DEFAULT 35B** — MTP K=1 (pinned; 116.5 tok/s), calibrated fp8 KV (128K), qwen3_coder agentic stack; requires :dev ≥ 2026-07-10 (atlas#287) |
| `qwen3.6-27b-nvfp4` | nvidia/Qwen3.6-27B-NVFP4 | single | **DEFAULT 27B** — dense hybrid SSM+Attn, MTP K=1 (pinned), bf16 KV, qwen3_coder agentic stack; requires :dev ≥ 2026-07-10 (atlas#287) |
| `qwen3.6-27b-w4a4-concurrency` | centml/Qwen3.6-27B-NVFP4-W4A4-mlpinf | single | **MANY-CLIENT 27B** — batch 128, MTP K=3, FP16 GDN h-state; 24.2/205.1/472.1 tok/s at C=1/16/128 and ahead of vLLM on all eight rungs. A THROUGHPUT config: the top two rungs invert on time-to-answer. Needs an image carrying the enterprise-concurrency merge |
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
├── nemotron-3-super/nemotron-3-super-120b-a12b-nvfp4.yaml
├── mistral-small-4/mistral-small-4-119b-nvfp4.yaml
└── minimax-m2.7/minimax-m2.7-nvfp4-ep2.yaml
```

sparkrun's recipe lookup is recursive within the `recipes` subtree, so the family-level grouping is purely cosmetic. Recipes are accessed by their file stem regardless of nesting.

## Hardware constraints captured in the recipes

Each recipe carries the production-validated KV/seq/MoE settings drawn from Atlas's `QUICKSTART.md`, the `scripts/sweep_all_models.sh` baseline, and the production `start-minimax-ep2.sh`/`start-ep2.sh` bring-up scripts. Notably:

- **Mistral Small 4** enforces `kv_cache_dtype: bf16` — FP8/NVFP4 KV destroys the MLA compressed latent (Atlas alpha-2.8 release announcement).
- **Qwen3-Coder-Next-FP8** requires `ssm_cache_slots: 0`, `oom_guard_mb: 1024`, and `kv_cache_dtype: bf16`.
- **122B EP=2** + **MiniMax M2.7 EP=2** carry matching `--speculative` / `--mtp-quantization` flags on both ranks (mismatched flags land MTP verify in the worker's SSM layer with no buffers allocated).
- **MiniMax M2.7 EP=2** is capped at `max_model_len: 12288` to fit the head's KV budget at `gpu_memory_utilization: 0.90` on the public `avarok/atlas-gb10:latest` image (live-validated 2026-05-08).

## Related

- Runtime: [`atlas` runtime in sparkrun](https://github.com/spark-arena/sparkrun) (PR #169)
- Engine: https://github.com/Avarok-Cybersecurity/atlas
- Docker image: [`avarok/atlas-gb10`](https://hub.docker.com/r/avarok/atlas-gb10)
- Discord: [Atlas-Inference](https://atlasinference.io)

## License

AGPL-3.0 — see [LICENSE](LICENSE). Matches the upstream [Atlas](https://github.com/Avarok-Cybersecurity/atlas) license.
