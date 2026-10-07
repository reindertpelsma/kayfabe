#!/bin/bash
# ★★★★★ w368 — THE NORTH STAR, RE-ASKED. The LLM workload has NOT been run since the
# sequential-multi-process defect was fixed (w366 orphan reclaim + w367 cap re-key).
# ⚠ The LLM workload runs `nvidia-smi` FIRST, so it was ALWAYS a >=2-process case — which
# is precisely what was broken until 3 hours ago. This is the first honest re-ask.
LOG=/root/w368.log
exec >"$LOG" 2>&1
echo "=== START $(date -Is) ==="
export PATH="$HOME/.cargo/bin:$PATH"
T=/workspace/bench/kayfabe; A="$T/scripts/bench/w329_arm.sh"
export KAYFABE_REPO=$T CARGO_TARGET_DIR=/workspace/bench/cargo-target-w330v
export KAYFABE_SHIM_FEATURES=host-isolates CARGO_BUILD_JOBS=10
cd "$T" || { echo "EXIT=90"; exit 90; }
git checkout -q cca2eb4b || { echo "EXIT=91"; exit 91; }
echo "HEAD=$(git rev-parse HEAD)"
echo "tracked dirt: [$(git status --porcelain --untracked-files=no)]"
touch crates/kayfabe-util/src/lib.rs
scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build >/dev/null 2>&1
QS=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 | grep -oE "kayfabe-rev:[0-9a-f]{40}" | sort -u)
case "$QS" in kayfabe-rev:cca2eb4b*) echo "GATE ok $QS";; *) echo "GATE REFUSED ($QS)"; echo "EXIT=93"; exit 93;; esac
export KAYFABE_GPU_NAME="NVIDIA GeForce RTX 3060"
export KAYFABE_GPU_SHORT_NAME="RTX 3060"
W329_WORKLOAD="w336_llm.sh run" timeout 3000 bash "$A" "$T" w368llm 1 \
   W329_WORKLOAD="w336_llm.sh run" >/dev/null 2>&1
P=/workspace/bench/run_w368llm1_probe.log; Q=/workspace/bench/run_w368llm1_qemu.log
echo "############ GRADED ############"
[ -s "$Q" ] || echo "⊘ NO QEMU LOG — THE BOOT NEVER HAPPENED"
sed -n "/=== w336 LLM/,\$p" "$P" 2>/dev/null | head -40
echo "--- ★★★★★ TOKENS ARE UN-FORGEABLE: LLM_TOKENS is the whole grade ---"
grep -E "^LLM_(TOKENS|OK|RC|MS|OUTCOME|TEXT)=" "$P" 2>/dev/null
echo "--- how many procs reached the device? (w367: 3 for 4 workloads) ---"
grep -oE "COMPLETION-DECLARE token=[^ ]* proc=[0-9]+" "$Q" 2>/dev/null | grep -oE "proc=[0-9]+" | sort -u | tr '\n' ' '; echo
echo "--- completions per proc (the discriminator) ---"
grep "COMPLETION-WATCH" "$Q" 2>/dev/null | grep -oE "proc=[0-9]+ .*(→ (NOT-)?OBSERVED)" | sed -E "s/.*(proc=[0-9]+).*(→ .*OBSERVED)/\1 \2/" | sort | uniq -c
echo "--- the two fixes: did they fire? ---"
echo "ORPHAN-RECLAIMED=$(grep -c 'ORPHAN-RECLAIMED' "$Q" 2>/dev/null) NOT-AN-ORPHAN=$(grep -c 'NOT AN ORPHAN' "$Q" 2>/dev/null)"
echo "--- host Xid (file must EXIST; empty grep is not zero) ---"
H=/workspace/bench/run_w368llm1_hostdmesg.log
if [ -f "$H" ]; then echo "hostdmesg_lines=$(wc -l < $H) Xid=$(grep -c Xid $H)"; else echo "⊘ NO HOST DMESG FILE — Xid UNMEASURED, not 0"; fi
echo "--- guest NVRM ---"; grep -ac NVRM /workspace/bench/run_w368llm1_dmesg.log 2>/dev/null
echo "=== DONE $(date -Is) ==="
echo "EXIT=0"
