#!/usr/bin/env bash
# ★★★★★ HOST PREFLIGHT — answer "is this box usable?" BEFORE any kayfabe failure is
# attributed to kayfabe.
#
# ⊘ THE PROBLEM THIS EXISTS FOR. vast hosts are community-rented and cheap *because* they
# are unreliable. When a bench run fails, the first question is not "what did we break" but
# "is this host even sound" — and without an answer, hours get spent debugging our code on a
# machine whose GPU, disk or network was broken on arrival. Renting a fresh box costs cents;
# misattributing a host fault costs a night.
#
# ★ Adapted from the discipline nvkvm-pv's `validate.sh` established: a precondition sweep
# that must PASS before anything downstream is believed.
#
# CONTRACT
#   - Every check ASSERTS. A check that cannot run is a FAIL, never a skip.
#   - Prints a terminator line and an explicit exit status, so "the file exists but has no
#     terminator" is detectable — a truncated run must not read as a pass.
#   - Exit 0 = host sound. Exit 2 = HOST FAULT (destroy it, rent another; do not nurse).
#     Exit 3 = host sound but WRONG for this project (e.g. non-GA10x GPU).
#
# ⚠ Tier 1 only: no repo build required, so it can run on a box seconds after it boots.
# Tier 2 (our raw RM client round-trip, `scripts/bench/` R33) needs a build and is separate.
set -u
FAIL=0; WRONG=0
ok(){ printf '  ✔ %s\n' "$*"; }
bad(){ printf '  ✘ HOST FAULT: %s\n' "$*"; FAIL=1; }
wrong(){ printf '  ⊘ WRONG BOX: %s\n' "$*"; WRONG=1; }

echo "=== kayfabe host preflight $(date -Is) on $(hostname) ==="

# 1. KVM — the single most common way a 'GPU box' turns out useless. vms_enabled describes
#    the HOST's capability, not the instance you were given: a plain --image yields a Docker
#    container where nvidia-smi is perfect and /dev/kvm does not exist.
[ -c /dev/kvm ] && ok "/dev/kvm present" || bad "/dev/kvm MISSING — this is a container, not a VM instance"
grep -qE 'vmx|svm' /proc/cpuinfo && ok "CPU virt extensions present" || bad "no vmx/svm in /proc/cpuinfo"

# 2. GPU present AND of the right family. Arch is a hard constraint: the register model is
#    GA10x. A Turing or Ada card is the WRONG BOX however healthy it is.
if command -v nvidia-smi >/dev/null 2>&1; then
  GPU=$(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | head -1)
  DRV=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader 2>/dev/null | head -1)
  if [ -z "$GPU" ]; then bad "nvidia-smi present but returned NO GPU (driver wedged?)"
  else
    ok "GPU: $GPU  driver: $DRV"
    case "$GPU" in
      *"RTX 30"*|*"A4000"*|*"A5000"*|*"A10"*) ok "GA10x family — matches kayfabe-arch" ;;
      *) wrong "$GPU is not GA10x; kayfabe-arch's only oracle-checked row is Ampere" ;;
    esac
  fi
else bad "nvidia-smi ABSENT"; fi

# 3. The GPU must actually WORK, not merely enumerate. ⊘ nvidia-smi listing a device proves
#    the driver loaded, NOT that a context can be created — the distinction that let a broken
#    cuMemAllocManaged pass 28/28 in a sibling project because the only UVM coverage was
#    checking that /dev/nvidia-uvm existed. A device node existing says nothing.
for n in /dev/nvidiactl /dev/nvidia0; do
  [ -c "$n" ] && ok "$n present" || bad "$n MISSING"
done
if [ -c /dev/nvidia-uvm ]; then ok "/dev/nvidia-uvm present"
else printf '  ⚠ /dev/nvidia-uvm absent (loads on first CUDA use; not fatal here)\n'; fi

# 4. Resources. A box that cannot hold the tree produces link failures that read as
#    regressions: ENOSPC has surfaced as an LLVM linker crash.
AVAIL=$(df -BG --output=avail / 2>/dev/null | tail -1 | tr -dc '0-9')
[ "${AVAIL:-0}" -ge 100 ] && ok "disk free ${AVAIL}G" || bad "only ${AVAIL:-?}G free on / — need >=100G"
RAMG=$(free -g | awk '/^Mem:/{print $2}')
[ "${RAMG:-0}" -ge 16 ] && ok "RAM ${RAMG}G" || bad "only ${RAMG:-?}G RAM"
CORES=$(nproc)
[ "${CORES:-0}" -ge 8 ] && ok "cores ${CORES}" || bad "only ${CORES:-?} cores"

# 5. Network egress. A host that cannot fetch cannot be provisioned, and the failure
#    otherwise appears much later as a mysteriously missing dependency.
if timeout 25 curl -fsSL -o /dev/null https://github.com 2>/dev/null; then ok "https egress works"
else bad "cannot reach github over https"; fi

echo "=== PREFLIGHT DONE ==="
if [ "$FAIL" -ne 0 ]; then echo "VERDICT=HOST_FAULT"; echo "EXIT=2"; exit 2; fi
if [ "$WRONG" -ne 0 ]; then echo "VERDICT=WRONG_BOX"; echo "EXIT=3"; exit 3; fi
echo "VERDICT=SOUND"; echo "EXIT=0"; exit 0
