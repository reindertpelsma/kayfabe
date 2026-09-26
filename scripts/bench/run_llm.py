#!/usr/bin/env python3
# ★★★★★ THE ONE LLM RUNNER — used by BOTH the guest lane and the host baseline (w713b).
#
# ⊘⊘ **It is one file on purpose.** Goal 9 is *"the LLM working, then AT PARITY"*, and parity is a
# RATIO. Two copies of a runner — one in the guest provisioner's heredoc, one written later for the
# host — would drift in prompt, token count, dtype or what the timer encloses, and the ratio would
# silently compare two different measurements. A ratio whose halves are not the same measurement is
# not a ratio.
#
# ⚠ `LLM_MS` times **`generate()` only**: not model load, not `cuInit`. That is the right basis for
# a guest-vs-host ratio and the WRONG number for "how long does the workload take". Both sides must
# use this file so the exclusion is identical on both.
#
# Every fact is its own greppable line, so a partial run is distinguishable from a failed one.
#
# ## ★ LONG-RUN / TIMELINE MODE (w828, v3-llm) — all OFF by default; the default run is unchanged.
#   LLM_TIMELINE=1  time each stage separately and per token, on the SAME clock on every lane:
#                   LLM_T_IMPORT_MS (import torch+transformers), LLM_T_CUINIT_MS (first context:
#                   torch.cuda.init + a 1-element tensor + synchronize), LLM_T_LOAD_MS
#                   (from_pretrained on CPU), LLM_T_TODEV_MS (.to(dev)), and per generate() run
#                   LLM_RUN <i> ms= ttft_ms= decode_tok_s= — ttft = generate() start to the first
#                   new token reaching the host; decode_tok_s = (n-1)/(t_last - t_first), i.e.
#                   steady-state, excluding load, cuInit and the first token. Per-token times come
#                   from a streamer whose put() receives next_tokens.cpu() — a host copy greedy
#                   decode already forces every step through its stop check.
#   LLM_MIN_NTOK=1  force exactly LLM_NTOK new tokens (min_new_tokens=NTOK): a long run must not
#                   end at EOS on one lane and not the other.
#   LLM_REPS=K      after the cold run, K more generate() runs in the same process (warm).
#   LLM_TEXT_SHA256 is always printed: the equality check for texts too long to eyeball.
import os, sys, time, hashlib
_T0 = time.time()
MODEL = os.environ.get('LLM_MODEL', 'Qwen/Qwen2-0.5B-Instruct')
NTOK  = int(os.environ.get('LLM_NTOK', '16'))
dev   = os.environ.get('LLM_DEVICE', 'cuda')
TL    = os.environ.get('LLM_TIMELINE', '0') == '1'
MINN  = os.environ.get('LLM_MIN_NTOK', '0') == '1'
REPS  = int(os.environ.get('LLM_REPS', '0'))
print('LLM_DEVICE=' + dev, flush=True)
rc, text, ntok, ms = 1, '', 0, -1.0
def _ms(t): return (time.time() - t) * 1000.0
try:
    import torch
    print('TORCH_CUDA_AVAILABLE=%s' % torch.cuda.is_available(), flush=True)
    print('TORCH_DEV_COUNT=%d' % torch.cuda.device_count(), flush=True)
    from transformers import AutoModelForCausalLM, AutoTokenizer
    if TL:
        print('LLM_T_IMPORT_MS=%.1f' % _ms(_T0), flush=True)
        print('LLM_TORCH=%s LLM_TRANSFORMERS=%s' % (torch.__version__,
              __import__('transformers').__version__), flush=True)
        if dev.startswith('cuda'):
            t = time.time()
            torch.cuda.init(); torch.zeros(1, device=dev); torch.cuda.synchronize()
            print('LLM_T_CUINIT_MS=%.1f' % _ms(t), flush=True)
            print('LLM_GPU=%s' % torch.cuda.get_device_name(0), flush=True)
    t = time.time()
    tok = AutoTokenizer.from_pretrained(MODEL)
    model = AutoModelForCausalLM.from_pretrained(MODEL, torch_dtype=torch.float16)
    if TL: print('LLM_T_LOAD_MS=%.1f' % _ms(t), flush=True)
    t = time.time()
    model = model.to(dev).eval()
    if TL:
        if dev.startswith('cuda'): torch.cuda.synchronize()
        print('LLM_T_TODEV_MS=%.1f' % _ms(t), flush=True)
    # ⊘⊘⊘ **`LLM_OK=1` MUST NOT MEAN "it ran". IT MUST MEAN "it ran WHERE WE ASKED".**
    #
    # ⚠ `.to('cuda')` raises when CUDA is absent, so *that* case already fails loudly. The hole is
    # the other one: `LLM_DEVICE=cpu` runs perfectly, reports `LLM_OK=1`, and produces a plausible
    # `LLM_MS` — and a debugging session that set it once and forgot would put a CPU number into a
    # GPU/GPU ratio. ⇒ The parameters are asked WHERE THEY ACTUALLY ARE, not where we requested.
    #
    # ★ This is the `correct_by_accident_under_a_temporary_condition` class: today nobody sets
    # `LLM_DEVICE`, so the default holds and the gap is invisible. The default is incidental.
    param_dev = next(model.parameters()).device.type
    print('LLM_PARAM_DEVICE=' + param_dev, flush=True)
    if param_dev != dev.split(':')[0]:
        raise RuntimeError('asked for %s, weights landed on %s' % (dev, param_dev))
    # ⊘ A ratio's two halves must be the same measurement. A non-CUDA run is a legitimate thing to
    # do deliberately, and an illegitimate thing to compare — so it is RUN and LOUDLY marked, never
    # refused outright.
    print('LLM_RATIO_ELIGIBLE=%d' % (1 if param_dev == 'cuda' else 0), flush=True)
    ids = tok('The capital of France is', return_tensors='pt').to(dev)
    kw = dict(max_new_tokens=NTOK, do_sample=False)
    if MINN: kw['min_new_tokens'] = NTOK
    class _Stamp:  # per-token host arrival times; put() #0 is the prompt
        def __init__(self): self.ts = []
        def put(self, v): self.ts.append(time.time())
        def end(self): pass
    for run in range(1 + (REPS if TL else 0)):
        st = _Stamp() if TL else None
        if st is not None: kw['streamer'] = st
        t0 = time.time()
        with torch.no_grad():
            out = model.generate(**ids, **kw)
        rms = (time.time() - t0) * 1000.0
        gen = out[0][ids['input_ids'].shape[1]:]
        if run == 0:
            ms = rms
            ntok = int(gen.shape[0])
            text = tok.decode(gen, skip_special_tokens=True)
        if st is not None:
            n = int(gen.shape[0]); tk = st.ts[1:]
            ttft = (tk[0] - t0) * 1000.0 if tk else -1.0
            dec = (len(tk) - 1) / (tk[-1] - tk[0]) if len(tk) > 1 and tk[-1] > tk[0] else -1.0
            same = tok.decode(gen, skip_special_tokens=True) == text
            print('LLM_RUN %d %s ntok=%d ms=%.1f tok_s=%.3f ttft_ms=%.1f decode_tok_s=%.3f same_text=%d'
                  % (run, 'cold' if run == 0 else 'warm', n, rms, n * 1000.0 / rms, ttft, dec,
                     1 if same else 0), flush=True)
    rc = 0
except Exception as e:
    print('LLM_EXC=%s: %s' % (type(e).__name__, e), flush=True)
print('LLM_TEXT=' + text.replace(chr(10), ' '), flush=True)
print('LLM_TEXT_SHA256=' + hashlib.sha256(text.encode()).hexdigest(), flush=True)
if TL: print('LLM_T_PROC_MS=%.1f' % _ms(_T0), flush=True)
print('LLM_OK=%d' % (1 if rc == 0 else 0), flush=True)
print('LLM_TOKENS=%d' % ntok, flush=True)
print('LLM_MS=%.1f' % ms, flush=True)
print('LLM_RC=%d' % rc, flush=True)
sys.exit(rc)
