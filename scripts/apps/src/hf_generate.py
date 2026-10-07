#!/usr/bin/env python3
# hf_generate.py — transformers greedy generate on the GPU; the text digest is compared host vs
# guest (same weights, same torch, greedy ⇒ the guest must produce the host's tokens).
import hashlib, os, sys, torch
from transformers import AutoModelForCausalLM, AutoTokenizer
m = os.environ.get("HF_MODEL", "Qwen/Qwen2-0.5B-Instruct")
tok = AutoTokenizer.from_pretrained(m)
model = AutoModelForCausalLM.from_pretrained(m, torch_dtype=torch.float16).cuda().eval()
ids = tok("Explain in three sentences why the sky is blue.", return_tensors="pt").to("cuda")
with torch.no_grad():
    out = model.generate(**ids, max_new_tokens=64, do_sample=False)
new = out[0, ids["input_ids"].shape[1]:].tolist()
text = tok.decode(new, skip_special_tokens=True)
print("HF_TEXT=" + text.replace("\n", " ")[:300])
print("HF_NTOK=%d" % len(new))
print("OUTSHA hf_generate " + hashlib.sha256(str(new).encode()).hexdigest()[:16])
print("CHECK hf_generate %s" % ("ok" if len(new) >= 16 and len(text.strip()) > 20 else "FAIL"))
