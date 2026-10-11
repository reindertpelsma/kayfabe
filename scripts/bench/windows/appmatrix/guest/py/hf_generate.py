# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Offline counterpart of scripts/apps/src/hf_generate.py; same model, prompt and greedy decode."""
import hashlib
import os
import torch
from transformers import AutoModelForCausalLM, AutoTokenizer

model_dir = os.environ['HF_MODEL']
tok = AutoTokenizer.from_pretrained(model_dir, local_files_only=True, trust_remote_code=False)
model = AutoModelForCausalLM.from_pretrained(
    model_dir, torch_dtype=torch.float16, local_files_only=True, trust_remote_code=False
).to('cuda:0').eval()
assert all(p.is_cuda for p in model.parameters()), 'model must run wholly on CUDA'
print('HF_DEVICE', torch.cuda.get_device_name(0), flush=True)
ids = tok('Explain in three sentences why the sky is blue.', return_tensors='pt').to('cuda:0')
with torch.no_grad():
    out = model.generate(**ids, max_new_tokens=64, do_sample=False)
assert out.is_cuda, 'generation must return a GPU tensor'
new = out[0, ids['input_ids'].shape[1]:].tolist()
text = tok.decode(new, skip_special_tokens=True)
print('HF_TEXT=' + text.replace('\n', ' ')[:300])
print('HF_NTOK=%d' % len(new))
print('OUTSHA hf_generate ' + hashlib.sha256(str(new).encode()).hexdigest()[:16])
ok = len(new) >= 16 and len(text.strip()) > 20
print('CHECK hf_generate %s' % ('ok' if ok else 'FAIL'))
raise SystemExit(0 if ok else 1)
