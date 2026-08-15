# NOTES: `qwen3.6-35b-a3b-nvfp4-diet-8k` (FAST)

Single-user GB10 **fast** memory-diet recipe for `nvidia/Qwen3.6-35B-A3B-NVFP4`.
Maximize **tok/s + host RAM**, not reasoning depth.

Evidence: spark-9f9e serve-flag matrix **2026-08-11** (3-run medians,
no-think short decode, ~256 completion tokens). Winner: `diet_40_8k`.

Sibling INTEL recipe (`qwen3.6-35b-a3b-nvfp4-intel`) is a separate stem —
do **not** collapse into one always-`--disable-thinking` doc.

## TL;DR (must not regress)

1. **Winner pins:** `--disable-thinking --gpu-memory-utilization 0.40
   --max-seq-len 8192 --speculative --num-drafts 1 --kv-cache-dtype fp8
   --kv-high-precision-layers auto --scheduling-policy slai
   --enable-prefix-caching --tool-call-parser qwen3_coder`
2. **Do NOT raise `--num-drafts`** on this MoE for single-user decode
   (K=2 ~80, K=3 ~57, ngram ~89 vs K=1 ~112).
3. **Nsight:** short decode is **MoE W4A16 expert GEMV-bound** (~66%
   top-3), not attention/host. **Do not promise KV-dtype flips as a
   short-decode tok/s win.**
4. Client: `enable_thinking: false`.

## Matrix leaderboard (spark-9f9e, 2026-08-11)

| tag | decode tok/s median | TTFT ms median | MemAvailable GB |
|---|---:|---:|---:|
| **diet_40_8k (winner)** | **111.91** | **141.9** | **40.79** |
| baseline_spec_k1 | 111.78 | 143.0 | 7.13 |
| no_tool_parser | 111.77 | 143.1 | 7.14 |
| mtp_vocab_100k | 111.5 | 144.0 | 7.16 |
| diet_55_32k | 111.13 | 142.9 | 31.1 |
| ngram_spec | 89.47 | 160.9 | 7.4 |
| spec_k2 (num-drafts 2) | 79.86 | 175.0 | 7.07 |
| spec_k3 (num-drafts 3) | 56.94 | 215.2 | 6.96 |

`diet_40_8k` runs: `[98.95, 113.36, 111.91]` → median ~112 tok/s.
Same speed as high-context baseline, ~6× more free host RAM.

## Do not raise `--num-drafts` on this MoE for single-user decode

Raising num-drafts hurt hard on this hybrid SSM model:

| drafts | decode tok/s median |
|---|---:|
| K=1 (pinned) | ~111.9 |
| K=2 | ~79.9 |
| K=3 | ~56.9 |
| ngram | ~89.5 |

Pin `num_drafts: 1`. MTP gate "ENABLED" is not evidence a deeper draft
is profitable — only a no-spec A/B is.

## Winner flags (parked Cmd)

```bash
spark serve nvidia/Qwen3.6-35B-A3B-NVFP4 \
  --port 8888 \
  --kv-cache-dtype fp8 --kv-high-precision-layers auto \
  --scheduling-policy slai --enable-prefix-caching --disable-thinking \
  --gpu-memory-utilization 0.40 --max-seq-len 8192 \
  --speculative --num-drafts 1 --tool-call-parser qwen3_coder
```

## Nsight short-decode (diet_40_8k) — GEMV wall

Sources (spark-9f9e):
- `~/atlas-perf/nsys/decode-diet40-wrap-20260812-024656.nsys-rep`
- `~/atlas-perf/nsys/FINDINGS.md`
- `~/atlas-perf/nsys/ncu/COMPUTE_FINDINGS.md` (ncu **blocked** —
  `ERR_NVGPUCTRPERM` / RmProfilingAdminOnly; Systems FINDINGS still valid)

**Verdict:** GPU/kernel-bound on MoE W4A16 expert GEMV — **not**
attention, **not** host. Profiler tax ~91 tok/s under nsys vs ~112
baseline.

| % GPU time | kernel |
|---:|---|
| 28.3 | `w4a16_gemv_batch2` |
| 24.4 | `moe_expert_gate_up_shared_batch2` |
| 13.8 | `moe_expert_silu_down_shared_batch2` |
| **~66** | **top-3 MoE/W4A16 combined** |
| 6.8 | `dense_gemv_bf16` |
| 4.2 | `gated_delta_rule_wy2` |
| **&lt;2** | **`paged_decode_attn_fp8` / `paged_decode_attn`** |

Supporting API profile: `cuStreamSynchronize` ~51% → waiting on GPU,
not launch-starved CPU. DtoH async API time notable but secondary.

**Recipe implications (do not soften):**
- Round-2 `--kv-cache-dtype nvfp4` may still free KV memory; it is **not**
  a promised short-decode tok/s win (attention ≪ 2% of kernel time).
- Real speed follow-up is **kernel** work on expert GEMV path (ncu
  roofline when privilege unlocked; fuse/retune gate_up+silu_down) —
  Sparker owns that lane, not this recipe PR.

## Prefill profile

One-liner: `~/atlas-perf/nsys/FINDINGS-prefill4k.md` — GEMM+GDN hot set, TTFT ~2.56s @ ~6.7k prompt (diet_40 cold prefill). Prefill ≠ decode GEMV wall; do not promise decode-kernel fixes for TTFT.

## Round-2 follow-ups for FAST (stubs only — unmeasured)

- `--max-batch-size 1` / `--max-num-seqs 1`
- `--kv-cache-dtype nvfp4` (memory experiment; see Nsight caveat)
- `--ssm-cache-slots 0`
- `--disable-tool-grammar true` / omit tool parser
- DFlash alt (`--dflash` + `z-lab/Qwen3.6-35B-A3B-DFlash`)

## Repro (after merge)

```bash
sparkrun run @atlas/qwen3.6-35b-a3b-nvfp4-diet-8k
```
