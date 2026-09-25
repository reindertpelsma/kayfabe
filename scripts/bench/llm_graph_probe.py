#!/usr/bin/env python3
# ★ Bisect the guest's CUDA-graph-arm failure (w828 [measured vh3 llm_g2]: `unspecified launch
# failure`, host Xid 13 SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE on a c7c0 channel, before capture).
# One STAGE per process (a CUDA error is sticky): run as `llm_graph_probe.py <stage>`.
#   side_small   a tiny elementwise kernel on a NON-default stream
#   side_model   an eager full-model forward on a non-default stream (DynamicCache)
#   static_pre   StaticCache prefill on the default stream
#   static_step  StaticCache prefill + one decode step on the default stream
#   capture      StaticCache prefill, then torch.cuda.graph capture of one step (no side warm-up)
# Prints PROBE_STAGE=<s> PROBE_OK=0|1 [PROBE_EXC=...].
import os, sys
stage = sys.argv[1]
MODEL = os.environ.get('LLM_MODEL', 'Qwen/Qwen2-0.5B-Instruct')
ok, exc = 0, ''
try:
    import torch
    x = torch.ones(1024, device='cuda'); torch.cuda.synchronize()
    if stage == 'side_small':
        s = torch.cuda.Stream()
        with torch.cuda.stream(s):
            y = x * 2 + 1
        s.synchronize(); assert float(y.sum()) == 3072.0
    else:
        from transformers import AutoModelForCausalLM, AutoTokenizer, StaticCache
        tok = AutoTokenizer.from_pretrained(MODEL)
        model = AutoModelForCausalLM.from_pretrained(MODEL, dtype=torch.float16).to('cuda').eval()
        ids = tok('The capital of France is', return_tensors='pt').input_ids.to('cuda')
        P = ids.shape[1]
        with torch.no_grad():
            if stage == 'side_model':
                s = torch.cuda.Stream(); s.wait_stream(torch.cuda.current_stream())
                with torch.cuda.stream(s):
                    o = model(input_ids=ids)
                s.synchronize(); print('PROBE_ARGMAX=%d' % int(o.logits[0, -1].argmax()))
            else:
                cache = StaticCache(config=model.config, max_cache_len=P + 64)
                o = model(input_ids=ids, past_key_values=cache,
                          cache_position=torch.arange(P, device='cuda'), use_cache=True)
                nxt = o.logits[:, -1, :].argmax(-1, keepdim=True)
                torch.cuda.synchronize(); print('PROBE_PREFILL_TOK=%d' % int(nxt))
                if stage in ('static_step', 'capture'):
                    pos = torch.tensor([P], device='cuda')
                    def step():
                        return model(input_ids=nxt, past_key_values=cache, cache_position=pos,
                                     position_ids=pos.unsqueeze(0), use_cache=True).logits
                    l = step(); torch.cuda.synchronize(); print('PROBE_STEP_TOK=%d' % int(l[0, -1].argmax()))
                    if stage == 'capture':
                        g = torch.cuda.CUDAGraph()
                        with torch.cuda.graph(g):
                            l2 = step()
                        g.replay(); torch.cuda.synchronize()
                        print('PROBE_REPLAY_TOK=%d' % int(l2[0, -1].argmax()))
    ok = 1
except Exception as e:
    exc = '%s: %s' % (type(e).__name__, str(e).splitlines()[0] if str(e) else '')
print('PROBE_STAGE=%s PROBE_OK=%d%s' % (stage, ok, (' PROBE_EXC=' + exc) if exc else ''), flush=True)
