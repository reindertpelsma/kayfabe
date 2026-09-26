#!/usr/bin/env bash
# ★★★★★ w734 — MEASURE THE TWO TERMS §3's ORDERING RESTS ON. Run ON the bench box.
#
#   usage: bash scripts/bench/w734_fbio_run.sh [tag]
#
# ## ⊘⊘⊘ WHAT THIS RUN IS FOR — and it is not a new arm
#
# `SINGLE_STORE_PLAN.md`'s *"§6 MUST PRECEDE §3"* and `THE_CONSTRAINTS.md` §w724c's *"there is
# no working intermediate — it does not boot"* are the SAME arithmetic:
#
#     how many bytes the host reads out of the framebuffer store  ÷  48 MiB/s
#
# `[surveyed w734]` **neither term had ever been measured on this path.** The numerator was
# `pages_swept` — a PAGE count, at six different page sizes of which three are not 4 KiB —
# turned into MiB by assumption. The denominator is quoted in both documents with no citation
# to a measurement of a CPU `memcpy` out of a device view of the reserved object.
#
# ⇒ This boot measures both, plus the term nobody had noticed the design also pays:
#
#   FB-IO          bytes AND distinct frames, per role (trap / walk-bar / walk-guest-pt /
#                  out-of-band / cpu-ce)
#   FB-IO-RATE     the rate the projection used, and whether it was MEASURED or ASSUMED
#   DEVICE_VIEW    rd/wr MiB/s over the reserved object, plus arm_us and rel_us PER VIEW
#   IDENTITY-*     the identity-window invariant at realize, and whether the guest stayed in
#   BAR1-BUDGET    §22 item 3's relation on this board
#
# ## ★★★ THE TWO TERMS ARE BOUNDED BY DIFFERENT THINGS — read both
#
#   bytes           ÷ the PCIe rate     ⇒ a large value means the boot gets SLOWER
#   distinct frames × one armed node    ⇒ a large value means it DOES NOT FIT, at any speed
#
# A device view is mapped from file offset **0 only** (`nvidia_mmap_helper` refuses
# `vm_pgoff != 0`), so a device-backed store needs one armed node per contiguous run, and an
# armed node costs host BAR1 — §22 item 3's measured 256 MiB, shared with the host driver.
# ⇒ `WALK_FRAMES` is the aperture term and it can point the opposite way from the byte count.
#
# ## ⊘ PRE-REGISTERED, so no number reads as the good one after the fact
#
#   Q1 THE CLIENT FIRST. `W392D_GUEST_OUTCOME=(P)`, `THREADS 8 of 8`. This boot changes no
#      behaviour — the census is counters and the probe is bounded — so anything but (P) makes
#      every number below a fact about a broken boot.
#   Q2 THE CENSUS EXISTS AND IS NOT VACUOUS. `⊘⊘ VACUOUS` means the counters were not reached,
#      NOT that the store served nothing: `kbusVerifyBar2` writes and reads it before the
#      guest's first instruction. A missing FB-IO line is UNMEASURED.
#   Q3 THE RATE IS `⇐ MEASURED`. `⇐ ASSUMED` means the device-view probe did not run, which is
#      NOT a slow rate — and a projection against the inherited 48 MiB/s is exactly the
#      derivation this run exists to replace.
#   Q4 THE VERDICT, computed from Q2 and Q3 and written down BEFORE the boot:
#        WALK seconds  < ~5 s   ⇒ the walk's byte cost does not block the switch
#        WALK seconds  > ~60 s  ⇒ it does, and §6 step 3 (the kernel owning reachability) is
#                                 a PREREQUISITE of §3 rather than a follow-on
#        WALK_FRAMES × 4 KiB + our headroom ≤ host BAR1 ⇒ the aperture fits; otherwise the
#        design needs recycling, which is a different design (§22 item 3 says so).
#      ⚠ Both thresholds are stated here, in advance, and neither is derived from the result.
#
# ## Traps encoded inline
#
# - ★★ `pgrep -x qemu-system-x86_64` can NEVER match (/proc/PID/comm truncates at 15).
# - ★★ the kill goes on a line of ITS OWN (nvkvm-pv 2026-08-17: a later word naming the binary
#   makes `pkill -f` match its own shell and everything after it silently never runs).
# - ★ `grep -c` on its own line, never piped into `grep -q` (SIGPIPE + pipefail = 141).
# - ★ fields are cut out of the FB-IO line itself, never grepped from the log: `frames=` and
#   `trap` are common words and w731/w732 both lifted a field off the wrong subsystem's census.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO=${KAYFABE_REPO:-/root/kayfabe}
BENCH=${BENCH_DIR:-/workspace/bench}
TAG=${1:-w734}
cd "$REPO" || { echo "⊘ no repo at $REPO"; exit 2; }

echo "=== W734 FB-IO RUN $(date -Is) tag=$TAG ==="
echo "TREE_REV=$(git rev-parse HEAD)"

pkill -f '[q]emu-system-x86'
sleep 3

# ⊘ `KAYFABE_DEVICE_VIEW=on` is the variable this run adds to §6's boot: it is what makes the
# crossing probe — and therefore the RATE probe beside it — run at all. Everything else is
# `single_store_e6_boot.sh`'s `on` arm verbatim, so the byte census is taken on a boot whose
# behaviour is already characterised.
# ⊘ The value is `probe`, NOT `on`. `[measured w734, first boot]` `=on` refused the device by
# name — *"the only values are `off` (the default) and `probe`"* — which is the gate working:
# arming it makes this process hold a `/dev/nvidia<N>`, and a typo must not decide that.
export KAYFABE_DEVICE_VIEW=probe
PREFIX="$TAG" SHADOW=on bash "$SRC_DIR/single_store_e6_boot.sh" 2>&1 | tee "$BENCH/w734_run.log"

Q="$BENCH/run_${TAG}_qemu.log"
D="$BENCH/run_${TAG}_probe.log"

echo
echo "=== ★★★★★ W734 — THE TWO TERMS ==="
echo "--- Q1 THE CLIENT (graded first) ---"
grep -a 'W392D_GUEST_OUTCOME=' "$D" 2>/dev/null | tail -1 | cut -c1-120
grep -a 'THREADS ' "$D" 2>/dev/null | tail -1 | cut -c1-90

echo "--- Q2 THE BYTE AND FRAME CENSUS ---"
n_io=$(grep -ac 'kayfabe: FB-IO ' "$Q" 2>/dev/null)
echo "W734-FBIO-LINES=${n_io:-0}  (0 ⇒ UNMEASURED, which is NOT 'the store served nothing')"
IO=$(grep -ao 'FB-IO .\{0,1400\}' "$Q" 2>/dev/null | tail -1)
printf '%s\n' "$IO" | fold -w 200
f() { printf '%s' "$IO" | grep -ao "$1" | tail -1; }
echo "W734-TOTAL=$(f 'TOTAL=[0-9.]*MiB')"
echo "W734-WALK=$(f 'WALK=[0-9.]*MiB')"
echo "W734-WALK-FRAMES=$(f 'WALK_FRAMES=[0-9]*')"
echo "W734-VACUOUS=$(printf '%s' "$IO" | grep -aoc 'VACUOUS')  (1 ⇒ the instrument was not reached)"

echo "--- Q3 THE RATE, AND ITS PROVENANCE ---"
grep -a 'kayfabe: FB-IO-RATE' "$Q" 2>/dev/null | tail -1 | cut -c1-200
echo "--- the device-view probe's own four numbers ---"
grep -ao 'DEVICE_VIEW=.\{0,320\}' "$Q" 2>/dev/null | tail -1

echo "--- Q4 THE IDENTITY WINDOW, both moments ---"
grep -ao 'IDENTITY-WINDOW .\{0,300\}' "$Q" 2>/dev/null | tail -1
grep -ao 'IDENTITY-REACHED .\{0,300\}' "$Q" 2>/dev/null | tail -1

echo "--- §22 item 3's relation on this board ---"
grep -ao 'BAR1-BUDGET .\{0,260\}' "$Q" 2>/dev/null | tail -1

echo "--- ★ the arena's own span, for cross-checking IDENTITY-REACHED ---"
grep -ao 'arena\[pages [^]]*\]' "$Q" 2>/dev/null | tail -1

echo "--- ★ host Xid, if any (a fault makes every number above suspect) ---"
grep -a 'HOST_DMESG_XID' "$BENCH/w734_run.log" 2>/dev/null | tail -2 | cut -c1-200

echo "=== W734 END $(date -Is) ==="
