#!/usr/bin/env bash
# §14.28 follow-up: which of the ids the port still refuses is NECESSARY for cuInit?
# ⊘ This instrument measures NECESSITY only — it subtracts one answer from a WORKING
# system. It can never enumerate sufficiency, which is exactly the mistake §14.27's
# matrix invited and this boot refuted.
cd /workspace/bench
run() { # $1 = label, rest = env
  local label=$1; shift
  local out
  out=$(env LD_PRELOAD=/workspace/bench/cuda_ioctl_trace.so NVTRACE_FILE=/dev/null "$@" \
        ./cuinit_probe 2>&1 | grep -aE "^cuInit\(0\)" | head -1)
  printf '%-34s %s\n' "$label" "${out:-<no cuInit line>}"
}
run "BASELINE (nothing forced)"
for c in 20810110 208f1105 20809009 2080014b 20800157 20801357 2080a612 2080a618 2080012b 00800294 20810108; do
  run "refuse CTRL 0x$c" NVFAULT_CTRL=0x$c
done
for a in 70 c36f 402c 208f; do
  run "refuse ALLOC 0x$a" NVFAULT_ALLOC=0x$a
done
