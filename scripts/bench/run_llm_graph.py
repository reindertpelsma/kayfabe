#!/usr/bin/env python3
# ★ DIAGNOSTIC ARM (w828, v3-llm) — the same model and prompt as run_llm.py, but the one-token
# decode step is CAPTURED ONCE as a CUDA graph and REPLAYED per token (StaticCache, greedy argmax
# inside the graph). ⊘ NOT the parity workload and never a substitute for it: it exists to separate
# "per-launch submission cost" from everything else. An eager HF decode step is ~1000 kernel
# launches (≈1000 doorbells); a graph replay is a handful. If the guest/host ratio of THIS arm is
# near 1 while run_llm.py's is not, the eager gap is per-launch (doorbell) cost, by construction.
#   env: LLM_MODEL, LLM_NTOK (default 512), LLM_REPS (warm runs after the cold one, default 1)
# Prints LLM_GRAPH_RUN <i> ntok= ms= tok_s= ttft_ms= decode_tok_s=, LLM_TEXT_SHA256, LLM_OK.
import os, sys, time, hashlib
MODEL = os.environ.get('LLM_MODEL', 'Qwen/Qwen2-0.5B-Instruct')
NTOK = int(os.environ.get('LLM_NTOK', '512'))
REPS = int(os.environ.get('LLM_REPS', '1'))
rc, text = 1, ''
try:
    import torch
    from transformers import AutoModelForCausalLM, AutoTokenizer, StaticCache
    dev = 'cuda'
    tok = AutoTokenizer.from_pretrained(MODEL)
    model = AutoModelForCausalLM.from_pretrained(MODEL, dtype=torch.float16).to(dev).eval()
    print('LLM_PARAM_DEVICE=' + next(model.parameters()).device.type, flush=True)
    ids = tok('The capital of France is', return_tensors='pt').input_ids.to(dev)
    P = ids.shape[1]
    L = P + NTOK + 8
    cache = StaticCache(config=model.config, max_cache_len=L)
    s_tok = torch.zeros((1, 1), dtype=torch.long, device=dev)
    s_pos = torch.zeros((1,), dtype=torch.long, device=dev)
    s_out = torch.zeros((1, 1), dtype=torch.long, device=dev)

    def step():
        o = model(input_ids=s_tok, past_key_values=cache, cache_position=s_pos,
                  position_ids=s_pos.unsqueeze(0), use_cache=True)
        s_out.copy_(o.logits[:, -1, :].argmax(-1, keepdim=True))

    def prefill():
        cache.reset()
        o = model(input_ids=ids, past_key_values=cache,
                  cache_position=torch.arange(P, device=dev), use_cache=True)
        return o.logits[:, -1, :].argmax(-1, keepdim=True)

    graph = None
    for run in range(1 + REPS):
        with torch.no_grad():
            t0 = time.time()
            nxt = prefill()
            out = [int(nxt.item())]
            t_first = time.time()
            s_tok.copy_(nxt)
            if graph is None:
                # warm-up on a side stream (CUDA graph capture rules), then capture one step
                s_pos.fill_(P)
                st = torch.cuda.Stream(); st.wait_stream(torch.cuda.current_stream())
                with torch.cuda.stream(st):
                    for _ in range(3): step()
                torch.cuda.current_stream().wait_stream(st)
                graph = torch.cuda.CUDAGraph()
                with torch.cuda.graph(graph):
                    step()
                print('LLM_GRAPH_CAPTURE_MS=%.1f' % ((time.time() - t_first) * 1000.0), flush=True)
                # capture/warm-up wrote junk into cache slot P: rerun prefill for a clean state
                nxt = prefill(); out = [int(nxt.item())]; s_tok.copy_(nxt); t_first = time.time()
            for i in range(NTOK - 1):
                s_pos.fill_(P + i)
                graph.replay()
                out.append(int(s_out.item()))
                s_tok.copy_(s_out)
            t1 = time.time()
        n = len(out)
        ms = (t1 - t0) * 1000.0
        dec = (n - 1) / (t1 - t_first) if t1 > t_first else -1.0
        t = tok.decode(out, skip_special_tokens=True)
        if run == 0: text = t
        print('LLM_GRAPH_RUN %d %s ntok=%d ms=%.1f tok_s=%.3f ttft_ms=%.1f decode_tok_s=%.3f same_text=%d'
              % (run, 'cold' if run == 0 else 'warm', n, ms, n * 1000.0 / ms,
                 (t_first - t0) * 1000.0, dec, 1 if t == text else 0), flush=True)
    rc = 0
except Exception as e:
    print('LLM_EXC=%s: %s' % (type(e).__name__, e), flush=True)
print('LLM_TEXT=' + text.replace(chr(10), ' ')[:200], flush=True)
print('LLM_TEXT_SHA256=' + hashlib.sha256(text.encode()).hexdigest(), flush=True)
print('LLM_OK=%d' % (1 if rc == 0 else 0), flush=True)
sys.exit(rc)
