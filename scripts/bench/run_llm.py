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
import os, sys, time
MODEL = os.environ.get('LLM_MODEL', 'Qwen/Qwen2-0.5B-Instruct')
NTOK  = int(os.environ.get('LLM_NTOK', '16'))
dev   = os.environ.get('LLM_DEVICE', 'cuda')
print('LLM_DEVICE=' + dev, flush=True)
rc, text, ntok, ms = 1, '', 0, -1.0
try:
    import torch
    print('TORCH_CUDA_AVAILABLE=%s' % torch.cuda.is_available(), flush=True)
    print('TORCH_DEV_COUNT=%d' % torch.cuda.device_count(), flush=True)
    from transformers import AutoModelForCausalLM, AutoTokenizer
    tok = AutoTokenizer.from_pretrained(MODEL)
    model = AutoModelForCausalLM.from_pretrained(MODEL, torch_dtype=torch.float16)
    model = model.to(dev).eval()
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
    t0 = time.time()
    with torch.no_grad():
        out = model.generate(**ids, max_new_tokens=NTOK, do_sample=False)
    ms = (time.time() - t0) * 1000.0
    gen = out[0][ids['input_ids'].shape[1]:]
    ntok = int(gen.shape[0])
    text = tok.decode(gen, skip_special_tokens=True)
    rc = 0
except Exception as e:
    print('LLM_EXC=%s: %s' % (type(e).__name__, e), flush=True)
print('LLM_TEXT=' + text.replace(chr(10), ' '), flush=True)
print('LLM_OK=%d' % (1 if rc == 0 else 0), flush=True)
print('LLM_TOKENS=%d' % ntok, flush=True)
print('LLM_MS=%.1f' % ms, flush=True)
print('LLM_RC=%d' % rc, flush=True)
sys.exit(rc)
