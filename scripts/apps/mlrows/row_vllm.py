#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""vLLM row workload (runs in the row's venv; stdlib + vllm). Measures, prints ONE `MLROW_JSON` line, decides nothing (mlrows_lib.judge_vllm does).

STATUS: LIVE, 2026-11-17 (docs/design/V3_OPEN_ML_ROWS.md). The offline engine, `LLM(...)`, on a small Apache-2.0 model from a local directory (the runner fetched it at its pinned
revision), temperature 0, fixed prompts. What it asks of the driver: the engine core in its own process (a second CUDA context on the die), a profiling forward that sizes the KV
cache from the memory the device reports (gpu_memory_utilization 0.9, the vLLM default), torch.compile and CUDA graph capture for the engine's whole list of batch sizes, then
greedy decoding in batches of different sizes. The same batch is generated twice (the tokens must be identical) and one prompt at a time (batch invariance, recorded only).
The parent process never creates a CUDA context: the runner samples device memory with nvidia-smi, so the engine core is the only CUDA client of the die.
"""
import argparse
import gc
import hashlib
import json
import sys
import time

PROMPTS = ["The capital of France is", "Once upon a time", "def fibonacci(n):", "The three primary colors are", "In 1969, humans first",
           "Water boils at a temperature of", "The quick brown fox", "A neural network is"]
MAX_TOKENS = 32


def emit(res):
    print("MLROW_JSON " + json.dumps(res, sort_keys=True, separators=(",", ":"), default=str), flush=True)


def canon_sha(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def graph_facts(llm):
    """The engine's own capture plan: mode and the list of batch sizes. Best effort across vLLM versions; None is recorded as None, never guessed."""
    cfg = None
    for path in (("llm_engine", "vllm_config"), ("llm_engine", "engine_core", "vllm_config")):
        o = llm
        try:
            for p in path:
                o = getattr(o, p)
            cfg = o
            break
        except AttributeError:
            continue
    cc = getattr(cfg, "compilation_config", None)
    sizes = getattr(cc, "cudagraph_capture_sizes", None)
    mode = getattr(cc, "cudagraph_mode", None)
    return {"sizes": list(sizes) if sizes is not None else None, "mode": str(mode) if mode is not None else None}


def children():
    try:
        import psutil
        return sorted({p.name() for p in psutil.Process().children(recursive=True)})
    except Exception:  # noqa: BLE001
        return []


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model-dir", required=True)
    ap.add_argument("--negctl", action="store_true")
    a = ap.parse_args()
    t0 = time.time()
    import vllm
    from vllm import LLM, SamplingParams
    res = {"row": "vllm", "vllm": vllm.__version__, "model_dir": a.model_dir.rsplit("/", 2)[-2:], "prompts": len(PROMPTS), "max_tokens": MAX_TOKENS}
    try:
        import torch
        res["torch"] = torch.__version__
    except Exception:  # noqa: BLE001
        pass
    t = time.time()
    llm = LLM(model=a.model_dir, tokenizer=a.model_dir, dtype="float16", max_model_len=512, seed=0, gpu_memory_utilization=0.9)
    res["engine_start_s"] = round(time.time() - t, 1)
    sp = SamplingParams(temperature=0.0, max_tokens=MAX_TOKENS, seed=0)

    def gen(prompts):
        return [list(o.outputs[0].token_ids) for o in llm.generate(prompts, sp, use_tqdm=False)]

    t = time.time()
    first = gen(PROMPTS)
    second = gen(PROMPTS)
    res["gen_s"] = round(time.time() - t, 2)
    res["token_ids"] = first
    res["token_sha256"] = canon_sha(first)
    res["repeat_identical"] = first == second
    res["text_head"] = [o.outputs[0].text[:40] for o in llm.generate(PROMPTS[:2], sp, use_tqdm=False)]
    # batch sizes 1, 3 and 40 (the capture list has a graph for each size's next bucket): the same tokens are expected per prompt; recorded, not judged
    single = [gen([p])[0] for p in PROMPTS]
    res["unbatched_equal"] = single == first
    big = gen(PROMPTS * 5)
    res["big_batch_copies_equal"] = all(big[i::len(PROMPTS)] == [first[i]] * 5 for i in range(len(PROMPTS)))
    g = graph_facts(llm)
    res["cudagraph_n_sizes"] = len(g["sizes"]) if g["sizes"] is not None else None
    res["cudagraph_sizes_head"] = (g["sizes"] or [])[:12]
    res["cudagraph_mode"] = g["mode"]
    res["engine_children"] = children()
    res["wall_s"] = round(time.time() - t0, 1)
    emit(res)
    try:
        llm.llm_engine.engine_core.shutdown()
    except Exception:  # noqa: BLE001
        pass
    del llm
    gc.collect()


if __name__ == "__main__":
    main()
    sys.stdout.flush()
