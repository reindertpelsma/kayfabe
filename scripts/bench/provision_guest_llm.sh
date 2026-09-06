#!/usr/bin/env bash
# ★★★ Install the LLM workload INTO the guest image — venv, torch, transformers, Qwen2-0.5B.
#
# ⚠ WHY THIS FILE EXISTS: the w368–w374 LLM harness (`/opt/llm/run_llm.py`, tag `w368llm1`)
# lived ONLY on the box that ran it, and that box went `Connection refused`. This is the third
# time this campaign has paid for it. `VAST IS COMPUTE, NEVER STORAGE` — the harness is
# committed BEFORE the run it drives, not after.
#
# Runs against the POWERED-OFF bench guest, on the stock hypervisor over slirp. ~10-20 min,
# dominated by the torch wheel (~2.5 GB) and the model download.
set -uo pipefail
BENCH=/workspace/bench
MODEL=${LLM_MODEL:-Qwen/Qwen2-0.5B-Instruct}
say(){ echo "[$(date -Is)] $*"; }
say "GUEST_LLM_START model=$MODEL"

pgrep -x qemu-system-x86 >/dev/null && { say "⊘ a QEMU is already running — the bench is serialized, refusing"; exit 2; }

qemu-system-x86_64 -enable-kvm -m 8G -smp 8 -display none \
  -drive if=virtio,file="$BENCH/guest.qcow2",format=qcow2 \
  -netdev user,id=n0,hostfwd=tcp::2222-:22 -device virtio-net-pci,netdev=n0 \
  -serial file:"$BENCH/llmprov_serial.log" -daemonize -pidfile "$BENCH/llmprov.pid"
GS="ssh -i $BENCH/guest_key -p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=8 ubuntu@127.0.0.1"
for i in $(seq 1 40); do $GS true >/dev/null 2>&1 && break; sleep 5; done
$GS true >/dev/null 2>&1 || { say "⊘ guest never answered ssh"; exit 3; }
# ⚠ cloud-init's netplan apply drops established sessions — wait it out (see provision_bench_tree).
for i in $(seq 1 40); do
  case "$($GS 'cloud-init status 2>/dev/null | head -1' 2>/dev/null | tr -d '\r')" in *done*) break ;; esac; sleep 5
done
say "guest up"

say "installing python venv + torch (cu124 wheel) — the long pole"
$GS "sudo apt-get update -qq && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq python3-venv python3-pip" 2>&1 | tail -2
$GS "python3 -m venv /home/ubuntu/llmvenv 2>/dev/null; /home/ubuntu/llmvenv/bin/pip -q install --upgrade pip" 2>&1 | tail -2
$GS "/home/ubuntu/llmvenv/bin/pip -q install torch --index-url https://download.pytorch.org/whl/cu124" 2>&1 | tail -3
$GS "/home/ubuntu/llmvenv/bin/pip -q install transformers accelerate" 2>&1 | tail -3

# ★ The runner. Prints ONE grading line per fact, each on its own line and each
#   independently greppable, so a partial run is distinguishable from a failed one.
# ⊘ LLM_TOKENS is the whole grade — a token count cannot be forged by a copy, a fill, or a
#   completion we wrote ourselves. LLM_OK / LLM_TEXT are context, not the verdict.
$GS "sudo mkdir -p /opt/llm && sudo chown ubuntu:ubuntu /opt/llm"
$GS "cat > /opt/llm/run_llm.py <<'PY'
import os, sys, time
MODEL = os.environ.get('LLM_MODEL', '$MODEL')
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
PY"

say "pre-downloading the model on the HOST side of the guest (so the graded run is not a network test)"
$GS "HF_HOME=/opt/llm/hf /home/ubuntu/llmvenv/bin/python -c \"
from transformers import AutoModelForCausalLM, AutoTokenizer
AutoTokenizer.from_pretrained('$MODEL'); AutoModelForCausalLM.from_pretrained('$MODEL')
print('MODEL_CACHED=yes')\"" 2>&1 | tail -3

# ⚠ VERIFY BY CONTENT, and on the CPU — a CPU generate proves the model, tokenizer and
#   runner are all sound BEFORE the GPU is in the picture. If this fails, a later GPU
#   failure is unattributable.
say "CPU control run (attribution: proves the workload itself works)"
CPU=$($GS "cd /opt/llm && HF_HOME=/opt/llm/hf LLM_DEVICE=cpu LLM_NTOK=8 /home/ubuntu/llmvenv/bin/python run_llm.py 2>&1 | grep -E '^LLM_(TOKENS|OK|TEXT)='" 2>&1 | tr '\n' ' ')
say "CPU control: $CPU"
case "$CPU" in
  *LLM_OK=1*) say "★ CPU control PASSED — the workload is sound; any GPU failure is OURS" ;;
  *) say "⊘ CPU control FAILED — do not attribute a later GPU failure to the emulator"; $GS "sudo poweroff" >/dev/null 2>&1 & sleep 15; exit 4 ;;
esac

$GS "sudo poweroff" >/dev/null 2>&1 &
sleep 20
say "GUEST_LLM_DONE rc=0"
