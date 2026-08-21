#!/bin/bash
LOG=/root/w362.log
exec >"$LOG" 2>&1
echo "=== START $(date -Is) ==="
export PATH="$HOME/.cargo/bin:$PATH"
T=/workspace/bench/kayfabe; A="$T/scripts/bench/w329_arm.sh"
export KAYFABE_REPO=$T CARGO_TARGET_DIR=/workspace/bench/cargo-target-w330v
export KAYFABE_SHIM_FEATURES=host-isolates CARGO_BUILD_JOBS=10
REV=c2a3631e536c85bf00859e7b163d87d09dfdcc6f
cd "$T" || { echo "EXIT=90"; exit 90; }
git fetch -q /root/w362.bundle w337-gpu-name-seam:w362 && git checkout -q $REV || { echo "EXIT=91"; exit 91; }
echo "HEAD=$(git rev-parse HEAD)"
echo "tracked dirt: [$(git status --porcelain --untracked-files=no)]"
chmod +x "$T/scripts/bench/w362_seq.sh"
touch crates/kayfabe-util/src/lib.rs
scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build >/dev/null 2>&1
# ⚠ two stamp greps, opposite blind spots — disagreement is the signal
QS=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 | grep -oE "kayfabe-rev:[0-9a-f]{40}" | sort -u)
case "$QS" in kayfabe-rev:c2a3631e*) echo "GATE ok $QS";; *) echo "GATE REFUSED ($QS)"; echo "EXIT=93"; exit 93;; esac
export KAYFABE_GPU_NAME="NVIDIA GeForce RTX 3060"
export KAYFABE_GPU_SHORT_NAME="RTX 3060"
W329_WORKLOAD="w362_seq.sh run" timeout 2400 bash "$A" "$T" w362seq 1 \
   W329_WORKLOAD="w362_seq.sh run" >/dev/null 2>&1
P=/workspace/bench/run_w362seq1_probe.log; Q=/workspace/bench/run_w362seq1_qemu.log
echo "############ GRADED ############"
[ -s "$Q" ] || echo "NO QEMU LOG - THE BOOT NEVER HAPPENED"
sed -n "/=== w362 SEQUENTIAL/,\$p" "$P" 2>/dev/null | head -40
echo "--- ★ THE DISCRIMINATOR: completions OBSERVED vs NOT, per proc ---"
grep "COMPLETION-WATCH" "$Q" 2>/dev/null | grep -oE "proc=[0-9]+ chan=[0-9]+ .*(→ (NOT-)?OBSERVED)" \
  | sed -E "s/.*(proc=[0-9]+).*(→ .*OBSERVED)/\1 \2/" | sort | uniq -c
echo "--- ★ THE INSTRUMENT FIX: distinct sema pages dumped (w361 saw only ONE) ---"
grep -oE "SEMA-PAGE-SLOT gpa=0x[0-9a-f]+" "$Q" 2>/dev/null | sort -u
echo "--- SEMA-PAGE-ZERO (all-zero page = written nowhere observable) ---"
grep -c "SEMA-PAGE-ZERO" "$Q" 2>/dev/null
echo "--- doorbells by proc+engine ---"
grep -oE "DOORBELL-XLATE proc=[0-9]+ .*engine=[A-Za-z]+" "$Q" 2>/dev/null | sed -E "s/.*(proc=[0-9]+).*(engine=[A-Za-z]+)/\1 \2/" | sort | uniq -c
echo "--- negative control (must NOT discriminate; w359 pass=507/2457) ---"
echo "PUBCONFLICT=$(grep -c PUBCONFLICT $Q 2>/dev/null) already_joined=$(grep -c 'already joined' $Q 2>/dev/null)"
echo "--- host Xid (file must EXIST; empty grep is not zero) ---"
H=/workspace/bench/run_w362seq1_hostdmesg.log
if [ -f "$H" ]; then echo "hostdmesg_lines=$(wc -l < $H) Xid=$(grep -c Xid $H)"; else echo "⊘ NO HOST DMESG FILE — Xid is UNMEASURED, not 0"; fi
echo "--- guest NVRM ---"; grep -ac NVRM /workspace/bench/run_w362seq1_dmesg.log 2>/dev/null
echo "=== DONE $(date -Is) ==="
echo "EXIT=0"
