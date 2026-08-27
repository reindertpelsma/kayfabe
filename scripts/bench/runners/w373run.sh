#!/bin/bash
LOG=/root/w373.log
exec >"$LOG" 2>&1
echo "=== START $(date -Is) ==="
export PATH="$HOME/.cargo/bin:$PATH"
T=/workspace/bench/kayfabe; A="$T/scripts/bench/w329_arm.sh"
export KAYFABE_REPO=$T CARGO_TARGET_DIR=/workspace/bench/cargo-target-w330v
export KAYFABE_SHIM_FEATURES=host-isolates CARGO_BUILD_JOBS=10
REV=$(git rev-parse cca2eb4b)
cd "$T" || { echo "EXIT=90"; exit 90; }
git checkout -q $REV || { echo "EXIT=91"; exit 91; }
echo "HEAD=$(git rev-parse HEAD)"
echo "tracked dirt: [$(git status --porcelain --untracked-files=no)]"
chmod +x "$T/scripts/bench/w373_seq.sh"
touch crates/kayfabe-util/src/lib.rs
scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build >/dev/null 2>&1
# ⚠ two stamp greps, opposite blind spots — disagreement is the signal
QS=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 | grep -oE "kayfabe-rev:[0-9a-f]{40}" | sort -u)
case "$QS" in kayfabe-rev:cca2eb4b*) echo "GATE ok $QS";; *) echo "GATE REFUSED ($QS)"; echo "EXIT=93"; exit 93;; esac
export KAYFABE_GPU_NAME="NVIDIA GeForce RTX 3060"
export KAYFABE_GPU_SHORT_NAME="RTX 3060"
STEP_TIMEOUT=600 W329_WORKLOAD="w373_seq.sh run" timeout 3000 bash "$A" "$T" w373seq 1 \
   STEP_TIMEOUT=600 W329_WORKLOAD="w373_seq.sh run" >/dev/null 2>&1
P=/workspace/bench/run_w373seq1_probe.log; Q=/workspace/bench/run_w373seq1_qemu.log
echo "############ GRADED ############"
[ -s "$Q" ] || echo "NO QEMU LOG - THE BOOT NEVER HAPPENED"
sed -n "/=== w373 SEQUENTIAL/,\$p" "$P" 2>/dev/null | head -40
echo "--- ★ THE DISCRIMINATOR: completions OBSERVED vs NOT, per proc ---"
grep "COMPLETION-WATCH" "$Q" 2>/dev/null | grep -oE "proc=[0-9]+ chan=[0-9]+ .*(→ (NOT-)?OBSERVED)" \
  | sed -E "s/.*(proc=[0-9]+).*(→ .*OBSERVED)/\1 \2/" | sort | uniq -c
echo "--- ★★★★★ THE RE-KEYED CAP: did CAPPED fall, and did the CE Xid go? ---"
echo "SUPERSEDE_CAPPED=$(grep -c 'SUPERSEDE CAPPED' "$Q" 2>/dev/null) (w366 baseline: 202 NO-ROW / capped frames 8x293 in w362)"
grep -oE "SUPERSEDE CAPPED at [0-9]+ takeovers" "$Q" 2>/dev/null | sort | uniq -c | head -3
echo "0x1e00000 still fabricated? refused=$(grep 'THE INSTALL REFUSED' "$Q" 2>/dev/null | grep -c 'phys=0x1e00000 ') joined=$(grep '→ JOINED' "$Q" 2>/dev/null | grep -c 'fb_phys=0x1e00000 ')"
echo "--- ★ HOST Xid detail (predicted: CE@0x7e59c6000000 GONE, GR@0x2_0440f000 may REMAIN) ---"
H=/workspace/bench/run_w373seq1_hostdmesg.log; [ -f "$H" ] && grep -iE "xid" "$H" | sed -E "s/.*(ENGINE [A-Z0-9_]+).*faulted @ ([0-9a-fx_]+).*type ([A-Z_]+).*/\1 @ \2 \3/" | sort | uniq -c || echo "⊘ NO HOST DMESG FILE — Xid UNMEASURED, not 0"
echo "--- ★★★★★ DID THE RECLAIM FIRE? (the pre-registered falsifier) ---"
echo "ORPHAN-RECLAIMED=$(grep -c 'ORPHAN-RECLAIMED' "$Q" 2>/dev/null) NOT-AN-ORPHAN=$(grep -c 'NOT AN ORPHAN' "$Q" 2>/dev/null) NO-OP=$(grep -c 'ORPHAN-RECLAIM NO-OP' "$Q" 2>/dev/null)"
grep -oE "ORPHAN-RECLAIMED fb_phys=0x[0-9a-f]+" "$Q" 2>/dev/null | sort | uniq -c | head -10
echo "--- ⚠ PRECONDITION: did a SECOND proc exist? (w364 had ONE, voiding the fork) ---"
echo "procs seen: $(grep -oE 'COMPLETION-DECLARE token=[^ ]* proc=[0-9]+' "$Q" 2>/dev/null | grep -oE 'proc=[0-9]+' | sort -u | tr '\n' ' ')"
echo "refusals by proc: $(grep 'THE INSTALL REFUSED' "$Q" 2>/dev/null | grep -oE 'proc=[0-9]+' | sort | uniq -c | tr '\n' ' ')"
echo "--- ★★★★★ WHO NAMES THE REFUSED FRAME? (the three-way fork) ---"
grep -oE "FB-JOIN-NAMERS\[live=[0-9]+ retired=[0-9]+ ⇒ [A-Z-]+" "$Q" 2>/dev/null | sort | uniq -c
echo "--- ★★★★★ THE FOURTH OUTCOME, no longer silent ---"
grep -c "SUPERSEDE NO-ROW" "$Q" 2>/dev/null
grep -oE "SUPERSEDE NO-ROW fb_phys=0x[0-9a-f]+" "$Q" 2>/dev/null | sort | uniq -c | head -8
echo "--- retired census (corpses vs rows) ---"
grep -oE "RETIRED-FB-CENSUS corpses=[0-9]+ join_rows=[0-9]+" "$Q" 2>/dev/null | sort | uniq -c | head -6
echo "--- ★★★★★ THE FIX: did a dead proc give its FB joins back? ---"
grep -c "RETIRED-FB-RELEASE ★★★★★" "$Q" 2>/dev/null
grep "RETIRED-FB-RELEASE fb_phys" "$Q" 2>/dev/null | sort -u | head -12
echo "--- ★ the three blocking frames: still refused? (w362 = 48 each) ---"
for f in 0x400000 0x600000 0x800000; do echo "  $f refused=$(grep "THE INSTALL REFUSED" "$Q" 2>/dev/null | grep -c "phys=$f ") joined=$(grep "→ JOINED" "$Q" 2>/dev/null | grep -c "fb_phys=$f ")"; done
echo "--- ★ THE INSTRUMENT FIX: distinct sema pages dumped (w361 saw only ONE) ---"
grep -oE "SEMA-PAGE-SLOT gpa=0x[0-9a-f]+" "$Q" 2>/dev/null | sort -u
echo "--- SEMA-PAGE-ZERO (all-zero page = written nowhere observable) ---"
grep -c "SEMA-PAGE-ZERO" "$Q" 2>/dev/null
echo "--- doorbells by proc+engine ---"
grep -oE "DOORBELL-XLATE proc=[0-9]+ .*engine=[A-Za-z]+" "$Q" 2>/dev/null | sed -E "s/.*(proc=[0-9]+).*(engine=[A-Za-z]+)/\1 \2/" | sort | uniq -c
echo "--- negative control (must NOT discriminate; w359 pass=507/2457) ---"
echo "PUBCONFLICT=$(grep -c PUBCONFLICT $Q 2>/dev/null) already_joined=$(grep -c 'already joined' $Q 2>/dev/null)"
echo "--- host Xid (file must EXIST; empty grep is not zero) ---"
H=/workspace/bench/run_w373seq1_hostdmesg.log
if [ -f "$H" ]; then echo "hostdmesg_lines=$(wc -l < $H) Xid=$(grep -c Xid $H)"; else echo "⊘ NO HOST DMESG FILE — Xid is UNMEASURED, not 0"; fi
echo "--- guest NVRM ---"; grep -ac NVRM /workspace/bench/run_w373seq1_dmesg.log 2>/dev/null
echo "=== DONE $(date -Is) ==="
echo "EXIT=0"
