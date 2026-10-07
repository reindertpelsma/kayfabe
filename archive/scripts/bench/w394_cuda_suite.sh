#!/usr/bin/env bash
# ★★★★★ w394 — lane (1) "CUDA apps work": the nvkvm-pv driver-API suite, run in the
# Mode-2 guest and natively, with the SAME binaries and the SAME arguments.
#
# usage:  w394_cuda_suite.sh native            # host, GPU direct — the parity DENOMINATOR
#         w394_cuda_suite.sh guest  [tag]      # POST_CAPTURE_HOOK arm — the NUMERATOR
#
# ## Why these six and not the ten .cu kernels
# These are `dlopen(libcuda)` + embedded PTX. They need **no CUDA toolkit** in either
# machine, so the guest arm requires nothing staged beyond the binary itself. The 10 `.cu`
# kernels in that tree need `nvcc` and are out of scope until a toolkit is staged.
#
# ## ★★★ The grade is CORRECTNESS FIRST, PERFORMANCE SECOND — and they are separate verdicts
# w386 measured a boot printing `LLM_TOKENS=16` over garbage text, and the `4,64` bandwidth
# workload reports `FUNC_RC=0 ROWS=2` with zero Xids while producing `bad=65536`. A number
# that arrives is not a number that is right. So:
#   * `vector_add_test`, `matmul_test`, `big_memcpy_test`, `mem_bandwidth_probe` VERIFY their
#     output against a CPU-computed expectation and are graded on that verification.
#   * `gpu_bench` and `cuda_micro` are TIMING ONLY. ⊘ `gpu_bench` does not check a single
#     result byte — never read its GFLOP/s as evidence the arithmetic was right.
#
# ## Traps encoded here, each already paid for in this campaign
#   * every run writes a START marker and an explicit `RC=` terminator, so an empty section
#     is distinguishable from a section that never ran (a killed job and a running job look
#     identical if absence-of-output is the only check).
#   * no pipelines around the probes: `cmd | tail` makes `$?` **tail's**.
#   * the guest arm asserts reachability FIRST and emits `(E) UNMEASURED` rather than a
#     failure, because "we could not ask" is not "the answer was no".
set -uo pipefail
ARM="${1:?usage: w394_cuda_suite.sh <native|guest> [tag]}"
TAG="${2:-w394}"
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
KEY=/workspace/bench/guest_key
BIN=${W394_BIN:-/workspace/bench/w394bin}
TMO=${W394_TIMEOUT:-600}

# ---- arguments are FIXED and SHARED by both arms. Parity across different arguments is
#      not parity; it is two different experiments with one name. ------------------------
declare -a PROBES=(
  "vector_add_test|verify|"
  "matmul_test|verify|"
  "big_memcpy_test|verify|4096"
  "mem_bandwidth_probe|verify|16 4"
  "gpu_bench|timing|1024 5 200 200"
  "cuda_micro|timing|2000 500 16 500"
)

# ---- build (host side, both arms; the guest gets the binary, not a compiler) -----------
build() {
  mkdir -p "$BIN"
  local rc=0 n=0
  for spec in "${PROBES[@]}"; do
    local name="${spec%%|*}"
    if ! cc -O2 -o "$BIN/$name" "$SRC_DIR/probes/$name.c" -ldl -lm 2>"$BIN/$name.buildlog"; then
      echo "W394_BUILD_FAIL=$name"; sed -n '1,10p' "$BIN/$name.buildlog"; rc=1
    else n=$((n+1)); fi
  done
  echo "W394_BUILT=$n of ${#PROBES[@]}  RC=$rc"
  return $rc
}

run_one() {                    # run_one <name> <args...>
  local name="$1"; shift
  echo "--- W394_${ARM^^}_BEGIN $name [$*] $(date -Is)"
  local rc=0
  if [ "$ARM" = native ]; then
    timeout "$TMO" "$BIN/$name" $* 2>&1; rc=$?
  else
    timeout "$TMO" "$G" "/tmp/w394/$name $*" 2>&1; rc=$?
  fi
  echo "W394_${ARM^^}_RC_${name}=$rc"
  echo "--- W394_${ARM^^}_END $name $(date -Is)"
}

# ★★★★★ W394_ONLY — RUN EXACTLY ONE PROBE, BECAUSE BEING FIFTH IS ITSELF A TREATMENT.
#
# `[measured w394, boot w394g]` this suite ran six probes in one boot. Probes 1-4 passed;
# probes 5 and 6 died on `cuInit 999` with `NVRM: _kgspBootGspRm: unexpected WPR2 already up`.
# That is w370's 5th-DEVICE-OPEN wedge, and the suite reproduced it by accident — it had no
# idea it was a multi-open test.
#
# ⇒ A timing number measured from the 5th open is a number about the wedge. For the parity
# arm each probe therefore gets its OWN boot, and `W394_ONLY` is how a boot is told which.
# ⊘ The CORRECTNESS arm is deliberately left as one boot of six: there, four passes plus a
#   reproduction of a known wall is more information than four boots of one.
if [ -n "${W394_ONLY:-}" ]; then
  keep=()
  for spec in "${PROBES[@]}"; do
    [ "${spec%%|*}" = "$W394_ONLY" ] && keep+=("$spec")
  done
  if [ ${#keep[@]} -eq 0 ]; then
    echo "W394_OUTCOME=(E) ⊘ UNMEASURED — W394_ONLY=$W394_ONLY names no probe in this suite"
    exit 2
  fi
  PROBES=("${keep[@]}")
fi

echo "=== ★ w394 CUDA-apps suite — arm=$ARM tag=$TAG only=${W394_ONLY:-<all>} $(date -Is) ==="
build || echo "⊘ at least one probe did not compile; the sections below are only as complete as W394_BUILT"

if [ "$ARM" = guest ]; then
  if ! "$G" true >/dev/null 2>&1; then
    echo "W394_OUTCOME=(E) ⊘ UNMEASURED_GUEST_UNREACHABLE — nothing was asked, so nothing was answered"
    exit 0
  fi
  "$G" 'rm -rf /tmp/w394 && mkdir -p /tmp/w394' >/dev/null 2>&1
  for spec in "${PROBES[@]}"; do
    name="${spec%%|*}"
    [ -x "$BIN/$name" ] || continue
    scp -i "$KEY" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        -o LogLevel=ERROR -o ConnectTimeout=5 \
        "$BIN/$name" ubuntu@192.168.77.2:/tmp/w394/ >/dev/null 2>&1 \
      || echo "W394_STAGE_FAIL=$name"
  done
  echo "W394_STAGED=$("$G" 'ls /tmp/w394 | wc -l' 2>/dev/null)"
  echo "W394_GUEST_LIBCUDA=$("$G" 'ls -la /usr/lib/x86_64-linux-gnu/libcuda.so.1 2>/dev/null || echo MISSING' 2>&1 | tail -1)"
fi

for spec in "${PROBES[@]}"; do
  name="${spec%%|*}"; rest="${spec#*|}"; kind="${rest%%|*}"; args="${rest#*|}"
  [ -x "$BIN/$name" ] || { echo "W394_${ARM^^}_RC_${name}=NOTBUILT"; continue; }
  run_one "$name" $args
done
echo "=== W394_${ARM^^}_SUITE_DONE $(date -Is) ==="
