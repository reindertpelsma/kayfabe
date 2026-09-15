#!/usr/bin/env bash
# ★★★★★ SINGLE-STORE §6 STEP 1 — THE LIVE WALK SHADOW.
#
#   usage: PREFIX=<tag> SHADOW=<off|on|swap> [START_MB=<n>] bash single_store_e6_boot.sh
#
# `on`   (§6 step 1) — the walk kernel is run ALONGSIDE the host walk at every off-vCPU
#        page-table sweep and the two answers are compared by kind. Nothing is published
#        from the kernel.
# `swap` (§6 step 2) — the same, and where the two AGREE the kernel's answer is what gets
#        PUBLISHED. Every refusal falls back to the host walk and prints its own line.
#
# ⊘⊘ **BOTH ARMS ARE GRADED ON THE CENSUS AND THE CLIENT, NEVER ON PARITY.** The swap is
# observationally neutral BY CONSTRUCTION — it substitutes only where the two walkers agree,
# and under agreement the two leaf sets are the same set. ⇒ a parity number cannot tell a
# working swap from a decider nobody consulted, and reading one as evidence would be this
# tree's `a_green_test_can_hold_a_wall_in_place` with a new subject. The known-positive that
# the substitution reaches `AddressTable` at all lives offline, in
# `tests/tests/walk_swap_decides.rs`.
#
# ## ★★ PRE-REGISTERED OUTCOMES — written before any boot, so none reads as the good one
#
#   Q1 THE CLIENT, graded FIRST. `W392D_GUEST_OUTCOME=(P)` with `THREADS 8 of 8`.
#      ⊘ The shadow must cost the guest nothing. If this is not (P) on the armed arm and IS
#      on the control, the shadow is the difference and the census is a fact about a broken
#      boot.
#
#   Q2 THE CENSUS EXISTS. One `WALK-SHADOW` line at END OF RUN, on BOTH arms.
#      ⊘⊘ A MISSING LINE IS AN UNMEASURED BOOT. `WALK-SHADOW ⊘ DISARMED` is what tells the
#      control arm apart from a binary that predates this increment — which is why the binary
#      is checked BY CONTENT below and REFUSES.
#
#   Q3b ⊘⊘ **THE SWAP'S OWN VERDICT, WHICH IS SEPARATE FROM Q3's.** On `SHADOW=swap` read
#       `swap_armed=` / `decided=` / `fell_back[…]` and the `⇒ … SWAP …` sentence:
#         `★★★ SWAP LIVE`      ⇐ THE GATE for step 2: decided>0
#         `⊘⊘ SWAP VACUOUS`    the arm was on and the kernel decided NOTHING — every
#                              published leaf came from the host walk, exactly as before.
#                              ⚠ This prints BESIDE `★★★ AGREEMENT`, and a reader who stops
#                              at the agreement verdict reads an unproven swap as proven.
#         `⊘ SWAP DISARMED`    the arm was `on`, not `swap`.
#       ⊘ `fell_back[none]` does NOT mean nothing was refused: it means nothing was ever
#       OFFERED. Read `skipped[…]` before believing it.
#
#   Q3 THE GATE: a census that is **not vacuous** and has **no disagreements**.
#      Read the verdict token, and read it in this order:
#        `⊘⊘ VACUOUS — … never compared`   the shadow never ran. Read `skipped[…]`:
#             on_vcpu=N            every sweep was on a vCPU thread — the shadow declines
#                                  there BY DESIGN (a round trip would block a vCPU)
#             too_many_pages=N     the image budget; read `image[pages_max=…]`
#             unreadable_page=N    the byte source would not serve a page the host just read
#             isolate_refused=N    the isolate said no — its own stderr carries the sentence
#             report_truncated=N   the kernel ran out of run slots
#        `⊘⊘ VACUOUS — … both sides were EMPTY`  compared, and there was nothing to compare
#        `★★★ AGREEMENT`                          ⇐ THE GATE
#        `⊘⊘ N DISAGREEMENTS`                     read `by_kind[…]` and `first[…]`
#
#   Q4 THE SCOPE, which is printed in the line itself and is NOT optional reading:
#      `compared_flags=0xf` (aperture + read-only; volatile/privilege/atomic/KIND are decoded
#      by the KERNEL ONLY), and the kernel walks a RELOCATED COPY holding exactly the pages
#      the host walk visited — so `missing_in_kernel` is fully live and `extra_in_kernel` is
#      live only within those pages. `absent_edges=` is the measured size of that clipping.
#
#   Q5 THE COST: `image[pages_max= staged_bytes=]` — what the shadow put on the wire.
#
#   (E) no `WALK-SHADOW` line at all ⇒ UNMEASURED. Say where it stopped.
#
# ## Traps encoded inline
#
# - ★★ `pgrep -x qemu-system-x86_64` can NEVER match (/proc/PID/comm truncates at 15).
# - ★ `grep -c` on its own line, never piped into `grep -q`: a pipe closing early returns 141
#   under `pipefail` and MANUFACTURES a failure when the string IS there (measured w418).
# - ★ The shadow needs the scratchpad AND its CUDA arm. Arming only this one gets a SAID
#   refusal at REALIZE, not a silent nothing — grep for it.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
tag=${PREFIX:-e6a}
SHADOW=${SHADOW:-off}

# ⊘ The variable under test is exported HERE and nowhere else. The scratchpad and its CUDA
# arm are NOT the variable: the shadow cannot exist without them, so they are on for both
# arms and the two boots differ in exactly one thing.
export KAYFABE_WALK_SHADOW="$SHADOW"
export KAYFABE_SCRATCHPAD=on
export KAYFABE_SCRATCHPAD_CUDA=on
export KAYFABE_SCRATCHPAD_START_MB="${START_MB:-4096}"

export KAYFABE_ISOLATES=real KAYFABE_GUEST_RAM=memfd NVKVM_RAM_BACKEND=memfd \
       KAYFABE_FB_JOIN=shared KAYFABE_GUEST_RING=ring KAYFABE_GR_ROUTE=passthrough \
       KAYFABE_CE_EXECUTOR=host \
       NVKVM_RAM_MB=${NVKVM_RAM_MB:-16384} BOOT_TIMEOUT=${BOOT_TIMEOUT:-180}
export POST_CAPTURE_HOOK="${POST_CAPTURE_HOOK:-$SRC_DIR/w392d_mean_hook.sh}"

echo "=== SINGLE-STORE E6 BOOT $(date -Is) tag=$tag ==="
echo "KAYFABE_WALK_SHADOW=$KAYFABE_WALK_SHADOW KAYFABE_SCRATCHPAD=$KAYFABE_SCRATCHPAD KAYFABE_SCRATCHPAD_CUDA=$KAYFABE_SCRATCHPAD_CUDA KAYFABE_SCRATCHPAD_START_MB=$KAYFABE_SCRATCHPAD_START_MB"
echo "POST_CAPTURE_HOOK=$POST_CAPTURE_HOOK"

Q_BIN="$BENCH/qemu-build/qemu-system-x86_64"
BIN_REV=$(strings "$Q_BIN" 2>/dev/null | grep -ao 'kayfabe-rev:[0-9a-f]\{40\}' | head -1 | cut -d: -f2)
TREE_REV=$(git -C "${KAYFABE_REPO:-/root/kayfabe}" rev-parse HEAD 2>/dev/null)
echo "BINARY_REV=${BIN_REV:-UNKNOWN} TREE_REV=${TREE_REV:-UNKNOWN}"
if [ -n "$BIN_REV" ] && [ -n "$TREE_REV" ] && [ "$BIN_REV" != "$TREE_REV" ]; then
  echo "⊘⊘⊘ STALE BINARY — refusing to grade a run that does not contain the tree."
  [ "${KAYFABE_ALLOW_STALE_BINARY:-0}" = "1" ] || exit 3
  echo "    ⊘ KAYFABE_ALLOW_STALE_BINARY=1 — proceeding, as asked."
fi

# ★★★ CONTENT CHECK, not a stamp: a binary that predates this increment prints no
# `WALK-SHADOW` line at all, and "no line" would otherwise read as "the arm was off".
n_ws=$(strings "$Q_BIN" 2>/dev/null | grep -c 'WALK-SHADOW')
n_cu=$(strings "$Q_BIN" 2>/dev/null | grep -c 'SCRATCHPAD-CUDA AT')
echo "E6-CONTENT: walk_shadow_strings=$n_ws cuda_census=$n_cu (either 0 ⇒ an OLDER binary — STOP)"
if [ "$n_ws" -eq 0 ] || [ "$n_cu" -eq 0 ]; then
  echo "⊘ REFUSING TO GRADE: the binary predates §6 step 1's live half. Rebuild, then re-run."
  exit 4
fi
n_img=$(strings "$Q_BIN" 2>/dev/null | grep -c 'kayfabe-isolate-cuda')
echo "E6-IMAGE: cuda_image_symbols=$n_img (0 ⇒ built WITHOUT --features cuda-scratchpad ⇒ the shadow CANNOT run)"

if pgrep -x qemu-system-x86 >/dev/null 2>&1; then echo "⊘ a QEMU is running; refusing"; exit 3; fi

bash "$SRC_DIR/boot_capture.sh" "$tag" > "$BENCH/run_${tag}_driver.log" 2>&1
echo "boot_capture rc=$?"

Q="$BENCH/run_${tag}_qemu.log"; D="$BENCH/run_${tag}_probe.log"

echo "--- Q1 THE CLIENT (graded first; everything below is uninterpretable without it) ---"
echo "[client] $(grep -a 'W392D_GUEST_OUTCOME=' "$D" 2>/dev/null | tail -1 | sed 's/^ *//' | cut -c1-90)"
echo "[client] $(grep -a 'THREADS ' "$D" 2>/dev/null | tail -1 | sed 's/^ *//' | cut -c1-80)"
echo "[client] $(grep -a 'MEAN_FALSIFIER' "$D" 2>/dev/null | tail -1 | sed 's/^ *//' | cut -c1-80)"

echo "--- ★★★★★ Q2/Q3/Q4/Q5 THE SHADOW CENSUS (a missing line is UNMEASURED, not clean) ---"
n_wc=$(grep -ac 'WALK-SHADOW' "$Q" 2>/dev/null)
echo "E6-SHADOW-LINES=${n_wc:-0}  (0 ⇒ UNMEASURED)"
grep -ao 'WALK-SHADOW[^|]\{0,1400\}' "$Q" 2>/dev/null | tail -2
# ⊘⊘ **EVERY FIELD IS CUT OUT OF THE `WALK-SHADOW` LINE ITSELF, NOT GREPPED FROM THE LOG.**
# `[measured w731]` a bare `grep 'by_kind\[...\]'` matched a DIFFERENT subsystem's census
# (`by_kind[doorbell=242 mirror_fill=41 invalidate=1132 …]`) and a bare `grep -c VACUOUS`
# counted 306 unrelated lines — so the control arm's summary printed numbers that had nothing
# to do with the shadow and looked like data. A field lifted from the wrong line is worse than
# a missing one.
WS=$(grep -ao 'WALK-SHADOW compared=[^|]\{0,1400\}' "$Q" 2>/dev/null | tail -1)
field() { printf '%s' "$WS" | grep -ao "$1" | tail -1; }
echo "E6-COMPARED=$(printf '%s' "$WS" | grep -ao 'compared=[0-9]*' | head -1 | cut -d= -f2)"
echo "E6-DISAGREEMENTS=$(field 'disagreements=[0-9]*' | cut -d= -f2)"
echo "E6-BY-KIND=$(field 'by_kind\[[^]]*\]')"
echo "E6-SKIPPED=$(field 'skipped\[[^]]*\]')"
echo "E6-IMAGE-STATS=$(field 'image\[[^]]*\]')"
# ★★★ §6 step 2. ⊘ Cut out of the WALK-SHADOW line itself, never grepped from the log —
# `decided=` and `fell_back[` are common enough words to match another subsystem's census.
echo "E6-SWAP-ARMED=$(field 'swap_armed=[a-z]*' | cut -d= -f2)"
echo "E6-DECIDED=$(field 'decided=[0-9]*' | cut -d= -f2)"
echo "E6-FELL-BACK=$(field 'fell_back\[[^]]*\]')"
echo "E6-SWAP-VERDICT=$(printf '%s' "$WS" | grep -ao 'SWAP [A-Z]*' | tail -1)"
echo "--- ★★★★★ every WALK-SWAP FALLBACK, verbatim (a disagreement must be LOUD, not a census row) ---"
n_fb=$(grep -ac 'WALK-SWAP FALLBACK' "$Q" 2>/dev/null)
echo "E6-FALLBACK-LINES=${n_fb:-0}"
grep -a 'WALK-SWAP FALLBACK' "$Q" 2>/dev/null | head -8 | cut -c1-260
echo "--- ★ the decider's shape check, if a replacement was ever refused whole ---"
grep -a 'PT-SWEEP DECIDER REFUSED' "$Q" 2>/dev/null | head -3 | cut -c1-260
echo "E6-VERDICT=$(printf '%s' "$WS" | grep -aoc 'VACUOUS')  (1 ⇒ the census is VACUOUS; 0 ⇒ it is not)"
echo "--- ★ the per-refresh refusals, verbatim, if any fired ---"
grep -a 'WALK-SHADOW image refused\|WALK-SHADOW refresh refused\|WALK-SHADOW report' "$Q" 2>/dev/null | head -6 | cut -c1-300
echo "--- ★ the isolate's OWN stderr for the shadow verbs ---"
grep -a 'kayfabe-isolate: ⊘ WALK-SHADOW' "$Q" 2>/dev/null | head -6 | cut -c1-300

echo "--- ★★★★★ Q2c THE CROSS-THREAD PROBE — the one whose absence cost w731's first boot ---"
echo "⊘ A CUDA context is CURRENT PER THREAD. The bring-up runs on the isolate's startup"
echo "  thread and every request is served on a WORKER. Probes (a) and (b) run where the"
echo "  context already is, so they cannot see this; it must begin PASS."
grep -ao 'probe_other_thread=\"[^\"]*\"' "$Q" 2>/dev/null | tail -1

echo "--- Q2b THE CUDA CENSUS: the shadow cannot run without it ---"
echo "E4-CUDA_WALK=$(grep -ao 'CUDA_WALK=[A-Z_]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)"
echo "E1-RESERVATION=$(grep -ao 'reservation=[A-Z_]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)"
echo "E1-RESERVED_MB=$(grep -ao 'RESERVED_MB=[0-9]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)"
echo "--- ★ the SAID refusal, if the shadow was armed with no isolate to run in ---"
grep -a 'WALK-SHADOW AT REALIZE' "$Q" 2>/dev/null | head -2 | cut -c1-300

echo "--- the arena counter the previous session's plan turned on (reported, not depended on) ---"
grep -ao 'arena\[[^]]*\]' "$Q" 2>/dev/null | tail -1

# ★★★★★ **§3's FIRST PRECONDITION, MEASURED RATHER THAN ASSUMED** — does the reserved object
# contain every framebuffer address the guest ever names?
#
# The post-§3 window is specified as identity (`gpga_is_one_reserved_object.md`: *"an address
# is `X + gpga_offset`"*), which is only sound if `span_pages * 4096 <= RESERVED_MB << 20`.
# ⊘ `[w730]` `span_pages=3087533` = **12062 MiB** was read as *"the guest's RM puts its tables
# ~11.78 GiB up a 12 GiB board"* — true, and it is a number about the **ADVERTISED** size, not
# about the reservation. The advertised size is rebound to the reservation whenever one is held
# (`SCRATCHPAD FB-SIZE derived_from_reservation=`), so the tables move DOWN with it.
# ⇒ The invariant §3 must never break: **advertise more than was reserved and the identity
# window is impossible**, and relocation cannot be retired. This line is what would catch it.
SPAN=$(grep -ao 'span_pages=[0-9]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)
RMB=$(grep -ao 'RESERVED_MB=[0-9]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)
DERIVED=$(grep -ao 'derived_from_reservation=[0-9]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)
echo "E3-SPAN-PAGES=${SPAN:-UNSET} E3-RESERVED_MB=${RMB:-UNSET} E3-ADVERTISED_MB=${DERIVED:-UNSET}"
if [ -n "${SPAN:-}" ] && [ -n "${RMB:-}" ]; then
  python3 - "$SPAN" "$RMB" <<'PYEOF'
import sys
span, rmb = int(sys.argv[1]), int(sys.argv[2])
top, cap = span * 4096, rmb << 20
print(f"E3-IDENTITY-WINDOW={'FITS' if top <= cap else 'DOES_NOT_FIT'} "
      f"top={top} ({top/2**20:.1f} MiB) reserved={cap} ({rmb} MiB) "
      f"headroom={(cap-top)/2**20:.1f} MiB")
print("  ⊘ FITS = every framebuffer page the guest named this boot is inside the reserved "
      "object ⇒ §3's identity window is arithmetically possible for THIS workload. It is "
      "NOT a proof for every workload, and it says nothing about the promote/demote FAKE "
      "RANGE, which is specified and built by nobody.")
PYEOF
else
  echo "E3-IDENTITY-WINDOW=UNMEASURED ⊘ one of the two numbers is missing — this is not a pass"
fi

echo "--- Q5 WHAT THE GUEST PAID ---"
grep -ao 'TRAPWITNESS[^|]*' "$Q" 2>/dev/null | tail -1
grep -ao 'SLOW-SITES[^⊘]*' "$Q" 2>/dev/null | tail -1
grep -ao 'PT-SWEEP[^|]\{0,200\}' "$Q" 2>/dev/null | tail -1
echo "HOST_DMESG_XID=$(grep -ac 'Xid' "$BENCH/run_${tag}_hostdmesg.log" 2>/dev/null)"

echo "--- the guest's OWN first failure (NOT the last line: a re-boot attempt masks it) ---"
grep -a 'NVRM' "$BENCH/run_${tag}_dmesg.log" 2>/dev/null | head -8 | cut -c1-170
echo "=== SINGLE-STORE E6 END $(date -Is) tag=$tag ==="
