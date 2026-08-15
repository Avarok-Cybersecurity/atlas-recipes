# NOTES: `qwen3.6-35b-a3b-nvfp4-intel` (INTEL)

Sibling of the **fast** `qwen3.6-35b-a3b-nvfp4-diet-8k` recipe.
Maximize **intelligence**: (1) ability to explain itself and
(2) superhuman-quality results — vs the fast disable-thinking lane.

**Tok/s is not the success metric.** Canary pass/fail alone is not enough.

Measured on spark-9f9e **2026-08-11/12**. Canonical score sheet:
`~/atlas-intel-eval/out/SCORED.md` (scored 2026-08-12 03:09 UTC).

## Two-track layout

| stem | goal | thinking | tools | mem/seq |
|---|---|---|---|---|
| `…-diet-8k` | tok/s + host RAM | off | parser on | 8k / 0.40 |
| `…-intel` (this) | explanation + answer caliber | **on** | parser **on** | **32k / 0.55** |

Do not merge into one always-disable-thinking document.

## Locked serve flags (measured park)

```bash
spark serve nvidia/Qwen3.6-35B-A3B-NVFP4 \
  --port 8888 \
  --kv-cache-dtype fp8 --kv-high-precision-layers auto \
  --scheduling-policy slai --enable-prefix-caching \
  --gpu-memory-utilization 0.55 --max-seq-len 32768 \
  --speculative --num-drafts 1 \
  --tool-call-parser qwen3_coder
  # thinking ON: omit --disable-thinking
```

Client: `enable_thinking: true`.

Still pin `--num-drafts 1` (FAST Round-1: K=2/3 regress on this MoE).

**32k / 0.55 is locked for now.** Higher `max-seq-len` remains
**optional** later for long agent threads — **not** required by this
eval (needle ~3.3k worked).

## Success metrics (from SCORED.md)

| axis | score |
|---|---:|
| Explanation | **13/16** |
| Result (answer caliber) | **12/16** |
| Combined | **25/32** |
| Pass bar (≥3, neither axis 0) | **5/8 pass** (3/8 soft-fail at 2) |
| Thinking present | **8/8** (`reasoning_tokens` ~256–273) |

### Per-item

| id | expl | result | sum | finish | note |
|---|---:|---:|---:|---|---|
| math_hard_explain_01 | 2 | 1 | 3 | stop | theory OK; missed ANSWER_* lines |
| math_constraint_explain_02 | 2 | 2 | 4 | stop | full pass |
| code_subtle_bug_01 | 2 | 2 | 4 | stop | full pass |
| tool_json_plan_01 | 2 | 2 | 4 | stop | clean JSON / allowlisted tools |
| needle_recall_explain_01 | 2 | 2 | 4 | stop | strong needle @ ~3.3k |
| adversarial_ambiguous_01 | 1 | 1 | 2 | length | runaway content |
| expert_synthesis_01 | 1 | 1 | 2 | stop | soft dual-resident flirt |
| code_algo_invariant_01 | 1 | 1 | 2 | length | runaway / meta loop |

Hard human-trap fails: **none**. Soft miss on `expert_synthesis_01`.
Delivery failures (not KV): length-cap on adversarial + binary-search.

Secondary tok/s in SCORED.md is informational only — do not cite as
the reason to ship or reject this recipe.

## Binding limiter (not 32k KV headroom)

The binding failure mode is **max_tokens / runaway content after short
~256-token thinking**, not context window:

- `adversarial_ambiguous_01` and `code_algo_invariant_01` hit
  `finish_reason=length` with process text leaking into `content`
- One format omission (`ANSWER_*`) on doors math
- Next knobs: clearer stop after final answer / selective max_tokens —
  **not** raising max-seq first

## Repro (after merge)

```bash
sparkrun run @atlas/qwen3.6-35b-a3b-nvfp4-intel
```
