#!/usr/bin/env bash
# ★★★★★ w742 — REUSED VERBATIM AGAIN. Only `report()` greps and the binary content gate
#   changed (marked `w742 ADDITION, REPORTING ONLY`). No arm, threshold or boot step.
#   ⊘ Edited IN PLACE rather than copied, for the reason this file already states three
#   comments down: a second harness for the same two-arm job is the thing these notes exist
#   to prevent. The w742 prediction table lives in `SINGLE_STORE_PLAN.md`, committed before
#   the box existed; these greps only cut its rows out of the logs.
# ★★★★★ w740 — REUSED VERBATIM AGAIN. Only `report()` greps and the binary content gate
#   changed (marked `w740 ADDITION, REPORTING ONLY`). No arm, threshold or boot step.
# ★★★★★ w736 — THE CUT-A BOOT. Tests `SINGLE_STORE_PLAN.md`'s w735 pre-registered prediction.
# ★★★★★ w738 — REUSED VERBATIM FOR THE CUT-B BOOT. Same two arms, same one variable, same
#   order (control first). The ONLY change is three extra `grep`s in `report()`, marked
#   `w738 ADDITION, REPORTING ONLY`: cut B added `FB-DEMAND`, `DEVICE-FB-PORT` and
#   `premap[pt_faults=] arm[…]`, and a harness that does not cut them out would grade cut B
#   on cut A's fields. ⊘ No arm, threshold or boot step differs — a second harness for the
#   same job is what this comment exists to prevent.
#
#   usage: bash scripts/bench/w736_fbstore_run.sh [tag]     (run ON the bench box)
#
# ## ⊘ THE PREDICTION, quoted from the doc and NOT restated in my own words
#
#   | line | predicted |
#   |---|---|
#   | `DEVICE-FB` | `named=0 host_read_refused>=1` **or** `host_write_refused>=1`, verdict
#   |             | `⊘ HOST-SIDE ACCESSES WERE REFUSED` |
#   | `DEVICE-VIEW-PORT` | `armed=0 refused=0` ⇒ `⊘⊘ VACUOUS` |
#   | where it dies | the FIRST framebuffer access at all, expected to be `kbusVerifyBar2`'s
#   |               | write inside `RmInitAdapter` — before the guest's first instruction,
#   |               | NOT at a BAR1 translate |
#
#   ★ `named>0` is the INTERESTING result: a guest memslot over real video memory installed
#     before anything needed host-side bytes ⇒ the memslot half works without cut B.
#   ⊘ dying LATER is a finding: the store is reached later than the model says and cut B's
#     four named consumers are not the whole list.
#
# ## THE ORDER, and why the control goes FIRST
#
# The device arm is EXPECTED to fail, and a failing boot can leave the host GPU in a state
# (Xid) that would contaminate whatever ran after it. Running the control first means its
# `(P)` is a fact about a clean box; running it second would leave "the control failed
# because the device arm wedged the GPU" and "the binary is broken" indistinguishable.
#
# ⚠ Traps encoded inline (CLAUDE.md, measured):
#   - the kill is on a line of ITS OWN (nvkvm-pv 2026-08-17: a later word naming the binary
#     makes `pkill -f` match its own shell and everything after it silently never runs).
#   - `pgrep -x qemu-system-x86_64` can NEVER match (/proc/PID/comm truncates at 15).
#   - `grep -c` on its own line, never piped into `grep -q` (SIGPIPE + pipefail = 141).
#   - every field is cut out of ITS OWN census line, never grepped loose from the log
#     (w731/w732 both lifted a field off the wrong subsystem's census).
set -uo pipefail
REPO=${KAYFABE_REPO:-/root/kayfabe}
BENCH=${BENCH_DIR:-/workspace/bench}
TAG=${1:-w736}
cd "$REPO" || { echo "⊘ no repo at $REPO"; exit 2; }

echo "=== W736 FB-STORE CUT-A RUN $(date -Is) tag=$TAG ==="
echo "TREE_REV=$(git rev-parse HEAD)  BRANCH=$(git branch --show-current)"

# ---- rebuild WITH the cuda scratchpad: without it there is no reservation, hence no
# device-view port, and `enforce_device_store` would refuse `NO-DEVICE-PORT` — a plumbing
# refusal wearing the same clothes as the designed wall.
export KAYFABE_SHIM_FEATURES="host-isolates cuda-scratchpad"
echo "== KAYFABE_SHIM_FEATURES=$KAYFABE_SHIM_FEATURES"
QEMU_SRC=${KAYFABE_QEMU_SRC:-$BENCH/qemu-10.2.4}
QEMU_BUILD=${KAYFABE_QEMU_BUILD:-$BENCH/qemu-build}
[ -f "$QEMU_SRC/VERSION" ] || { echo "⊘ no hypervisor source tree at $QEMU_SRC"; exit 2; }
bash scripts/build_qom_shim.sh "$QEMU_SRC" "$QEMU_BUILD" > "$BENCH/w736_build.log" 2>&1
rc=$?
echo "SHIM_RC=$rc"
if [ "$rc" -ne 0 ]; then
  echo "⊘ BUILD FAILED — last 40 lines:"; tail -40 "$BENCH/w736_build.log"; exit 3
fi

Q_BIN="$QEMU_BUILD/qemu-system-x86_64"
n_ws=$(strings "$Q_BIN" 2>/dev/null | grep -c 'WALK-SHADOW')
n_img=$(strings "$Q_BIN" 2>/dev/null | grep -c 'kayfabe-isolate-cuda')
n_fs=$(strings "$Q_BIN" 2>/dev/null | grep -c 'FB-STORE AT REALIZE')
n_dfb=$(strings "$Q_BIN" 2>/dev/null | grep -c 'DEVICE-FB named=')
# ★★★ w738 — CHECK THE BINARY FOR **CUT B**, BY CONTENT, AND REFUSE.
# ⊘ `FB-DEMAND` and `DEVICE-FB-PORT` print UNCONDITIONALLY on both arms, so a boot that does
# not carry them is an OLDER binary, not a quiet boot — and every row of w738's prediction
# would then be graded against absence. This is the same gate `E6-CONTENT` already applies to
# the walk shadow, for the reason that gate exists.
n_fd=$(strings "$Q_BIN" 2>/dev/null | grep -c 'FB-DEMAND drains=')
n_fbp=$(strings "$Q_BIN" 2>/dev/null | grep -c 'DEVICE-FB-PORT drains=')
# ★★★ w740 — THE SAME GATE, FOR THIS CHANGE. `W740-USERD-ARM` prints UNCONDITIONALLY on both
# arms, so a boot that does not carry the string is an OLDER BINARY, not a quiet boot, and
# every w740 row would then be graded against absence. Same reason `E6-CONTENT` exists.
n_w740=$(strings "$Q_BIN" 2>/dev/null | grep -c 'W740-USERD-ARM trips=')
# ★★★ w742 — THE SAME GATE, FOR THIS CHANGE. `DEVICE-LEAF no_join_needed=` prints
# UNCONDITIONALLY on BOTH arms at teardown, so a boot without the string is an OLDER BINARY
# and every w742 row would be graded against absence.
n_w742=$(strings "$Q_BIN" 2>/dev/null | grep -c 'DEVICE-LEAF no_join_needed=')
# ★★★ w743 — THE SAME GATE, FOR THIS CHANGE. `W743-PREFLIGHT passes=` is appended to the
# w740 arming census, which prints UNCONDITIONALLY on BOTH arms at teardown. A boot without
# the string is an OLDER BINARY and every w743 row would be graded against absence — and the
# most dangerous of those rows is `passes=0`, which on a stale binary is indistinguishable
# from "the pre-flight ran and never had to refuse".
n_w743=$(strings "$Q_BIN" 2>/dev/null | grep -c 'W743-PREFLIGHT passes=')
echo "W736-CONTENT: walk_shadow=$n_ws cuda_image=$n_img fb_store=$n_fs device_fb=$n_dfb"
echo "W738-CONTENT: fb_demand=$n_fd device_fb_port=$n_fbp (either 0 ⇒ the binary predates CUT B)"
echo "W740-CONTENT: userd_arm=$n_w740 (0 ⇒ the binary predates w740 — its rows cannot be graded)"
echo "W742-CONTENT: device_leaf=$n_w742 (0 ⇒ the binary predates w742 — its rows cannot be graded)"
echo "W743-CONTENT: preflight=$n_w743 (0 ⇒ the binary predates w743 — its rows cannot be graded)"
if [ "$n_img" -eq 0 ]; then echo "⊘ no cuda-scratchpad in the binary — the port cannot arm. STOP."; exit 5; fi
if [ "$n_fs" -eq 0 ] || [ "$n_dfb" -eq 0 ]; then echo "⊘ the binary predates cut A. STOP."; exit 4; fi
if [ "$n_fd" -eq 0 ] || [ "$n_fbp" -eq 0 ]; then echo "⊘ the binary predates CUT B — w738's rows cannot be graded. STOP."; exit 6; fi
if [ "$n_w740" -eq 0 ]; then echo "⊘ the binary predates w740 — its rows cannot be graded. STOP."; exit 7; fi
if [ "$n_w742" -eq 0 ]; then echo "⊘ the binary predates w742 — its rows cannot be graded. STOP."; exit 8; fi
if [ "$n_w743" -eq 0 ]; then echo "⊘ the binary predates w743 — its rows cannot be graded. STOP."; exit 9; fi

report() {
  local tag="$1" arm="$2"
  local Q="$BENCH/run_${tag}_qemu.log" D="$BENCH/run_${tag}_probe.log"
  echo
  echo "######## W736 REPORT arm=$arm tag=$tag ########"
  echo "--- the switch's own line at realize ---"
  grep -a 'FB-STORE AT REALIZE' "$Q" 2>/dev/null | head -2 | cut -c1-400
  echo "--- ★★★ PREDICTION 1a: the DEVICE-FB census, VERBATIM ---"
  n_dl=$(grep -ac 'DEVICE-FB named=' "$Q" 2>/dev/null)
  echo "W736-DEVICEFB-LINES=${n_dl:-0}  (0 ⇒ UNMEASURED, not 'nothing happened')"
  DFB=$(grep -ao 'DEVICE-FB named=.\{0,700\}' "$Q" 2>/dev/null | tail -1)
  printf '%s\n' "$DFB" | fold -w 160
  f() { printf '%s' "$DFB" | grep -ao "$1" | tail -1; }
  echo "W736-NAMED=$(f 'named=[0-9]*')"
  echo "W736-RD-REFUSED=$(f 'host_read_refused=[0-9]*')"
  echo "W736-WR-REFUSED=$(f 'host_write_refused=[0-9]*')"
  echo "W736-OOR=$(f 'out_of_range=[0-9]*')"
  # ★★★ w738 ADDITION, REPORTING ONLY — cut B's own fields, which cut A's store did not have.
  # ⊘ Nothing here changes an arm, a threshold or a boot step; it cuts three censuses out of
  # the same logs the w736 run already produced. `read_served` is cut B's GATE (row 2).
  echo "W738-RD-SERVED=$(f 'read_served=[0-9]*')"
  echo "W738-WR-SERVED=$(f 'write_served=[0-9]*')"
  echo "W738-WANTED-RD=$(f 'wanted_by_read=[0-9]*')"
  echo "W738-WANTED-WR=$(f 'wanted_by_write=[0-9]*')"
  echo "--- ★★★ w738 ROW 3: the FB-DEMAND census (cut B's callers), VERBATIM ---"
  n_fd=$(grep -ac 'FB-DEMAND drains=' "$Q" 2>/dev/null)
  echo "W738-FBDEMAND-LINES=${n_fd:-0}  (0 ⇒ UNMEASURED — the line is printed on BOTH arms, so"
  echo "                                 a missing one is an older binary, not a quiet boot)"
  FD=$(grep -ao 'FB-DEMAND drains=.\{0,700\}' "$Q" 2>/dev/null | tail -1)
  printf '%s\n' "$FD" | fold -w 160
  g() { printf '%s' "$FD" | grep -ao "$1" | tail -1; }
  echo "W738-FD-DRAINS=$(g 'drains=[0-9]*')"
  echo "W738-FD-ARMED=$(g 'armed=[0-9]*')"
  echo "W738-FD-REFUSED=$(g 'refused=[0-9]*')"
  echo "W738-FD-DECLINED=$(g 'declined_on_vcpu=[0-9]*')"
  echo "W738-FD-NOPORT=$(g 'no_port=[0-9]*')"
  echo "W738-FD-RETRIED-OK=$(g 'read_retried_ok=[0-9]*')"
  echo "W738-FD-GAVE-UP=$(g 'read_gave_up=[0-9]*')"
  echo "--- ★★★ w738 ROWS 4/5: the DEVICE-FB-PORT census (cut B's mechanism), VERBATIM ---"
  n_pp=$(grep -ac 'DEVICE-FB-PORT ' "$Q" 2>/dev/null)
  echo "W738-DEVFBPORT-LINES=${n_pp:-0}"
  grep -ao 'DEVICE-FB-PORT .\{0,800\}' "$Q" 2>/dev/null | tail -1 | fold -w 160
  echo "--- ★★★ w738 ROW 6: premap's fault count and the arm-then-retry trip counts ---"
  grep -ao 'premap\[[^]]*\] arm\[[^]]*\]' "$Q" 2>/dev/null | tail -1
  echo "--- w738: the store's first MISSED-AN-ARMED-VIEW line (cut B's transient, not cut A's wall) ---"
  grep -a 'MISSED AN ARMED VIEW' "$Q" 2>/dev/null | head -2 | cut -c1-300
  echo "--- w738: PREMAP's SHORT-enumeration line, if item 4's shape fired at all ---"
  grep -a 'PREMAP ⊘⊘' "$Q" 2>/dev/null | head -2 | cut -c1-300
  echo "--- w738: the cut-B banner at realize (absent ⇒ no byte port was attached at all) ---"
  grep -a 'CUT B — a byte port is attached\|CUT A SHAPE — no byte port' "$Q" 2>/dev/null | head -2 | cut -c1-300
  # ★★★ w739 ADDITION, REPORTING ONLY — cut C's three numbers, cut from the SAME logs.
  # ⊘ Nothing here changes an arm, a threshold or a boot step. All three are printed on BOTH
  #   arms, which is the point: `FB-IO walk-guest-pt` was read as *"cut B item 2 is inert"*
  #   from the DEVICE arm alone, on a boot that died at 32.7 s with `pre_birth_pages=NO-BIRTH`
  #   — i.e. before any guest CUDA page table exists to walk. A number that is only ever
  #   captured on the arm that dies early cannot tell "never needed" from "never reached".
  echo "--- ★★★ w739 CUT C 1/3: the fill queue, and whether a REFUSED access ever asked ---"
  n_fq=$(grep -ac 'BAR-MIRROR FILLS' "$Q" 2>/dev/null)
  echo "W739-FILLS-LINES=${n_fq:-0}  (0 ⇒ UNMEASURED, not 'no fills')"
  grep -ao 'BAR-MIRROR FILLS .\{0,400\}' "$Q" 2>/dev/null | tail -1 | fold -w 160
  FQ=$(grep -ao 'BAR-MIRROR FILLS .\{0,200\}' "$Q" 2>/dev/null | tail -1)
  h() { printf '%s' "$FQ" | grep -ao "$1" | tail -1; }
  echo "W739-FILLS-QUEUED=$(h 'queued=[0-9]*')"
  echo "W739-FILLS-RUN=$(h 'run=[0-9]*')"
  echo "W739-FILLS-FROM-REFUSAL=$(h 'from_refusal=[0-9]*')   ⊘ arena MUST be 0 (no byte port)"
  # ★★★ w740 ADDITION, REPORTING ONLY — the two arming loops on the CeUtils doorbell path.
  # ⊘ Cut from the SAME log, on BOTH arms. The control's zeros are row 3 (provably inert) and
  #   are as much a result as the device arm's non-zeros; a grep that ran on one arm only is
  #   how w738's `walk-guest-pt[r=0]` came to mean "inert" when it meant "never reached".
  echo "--- ★★★ w740 1/3: the CeUtils arming census, VERBATIM, both arms ---"
  n_arm=$(grep -ac 'W740-USERD-ARM trips=' "$Q" 2>/dev/null)
  echo "W740-ARM-LINES=${n_arm:-0}  (0 ⇒ UNMEASURED, not 'no arming')"
  ARM=$(grep -ao 'W740-USERD-ARM .\{0,700\}' "$Q" 2>/dev/null | tail -1)
  printf '%s\n' "$ARM" | fold -w 160
  k() { printf '%s' "$ARM" | grep -ao "$1" | tail -1; }
  echo "W740-USERD-TRIPS=$(k 'W740-USERD-ARM trips=[0-9]*')"
  echo "W740-USERD-RECOVERED=$(k 'recovered=[0-9]*')"
  echo "W740-USERD-GAVEUP=$(k 'gave_up=[0-9]*')"
  echo "W740-USERD-REFUSED-NO-ARM=$(k 'refused_no_arm=[0-9]*')"
  echo "W740-CE-SUBMIT=$(printf '%s' "$ARM" | grep -ao 'W740-CE-SUBMIT-ARM .\{0,120\}' | tail -1)"
  # ★★★★★ w743 — THE PRE-FLIGHT, CUT OUT OF ITS OWN LINE. ⊘ Read `passes=` FIRST: `0` on the
  # device arm means the arm never executed and NO other field here is interpretable.
  # `blocked=` and w740's `blocked_by_progress=` are the two halves of one question — a
  # refusal the pre-flight caught moved nothing and lands in `trips=`; one it missed lands in
  # `blocked_by_progress=`.
  PF=$(printf '%s' "$ARM" | grep -ao 'W743-PREFLIGHT .\{0,200\}' | tail -1)
  echo "W743-PREFLIGHT=$PF"
  pff() { printf '%s' "$PF" | grep -ao "$1" | tail -1; }
  echo "W743-PASSES=$(pff 'passes=[0-9]*')      ★ 0 on arena is CORRECT; 0 on device is the finding"
  echo "W743-PROBED=$(pff 'probed=[0-9]*')"
  echo "W743-UNARMED=$(pff 'unarmed=[0-9]*')"
  echo "W743-BLOCKED=$(pff 'blocked=[0-9]*')    ★ each is a refusal that moved NOTHING"
  echo "W743-TRUNCATED=$(pff 'truncated=[0-9]*')  ⊘ non-zero ⇒ atomicity NOT proved for that submission"
  echo "--- ★★★ w740 2/3: did a submission ever recover after a drain? ---"
  grep -a 'CE-SUBMIT-ARMED token=' "$Q" 2>/dev/null | head -4 | cut -c1-300
  echo "--- ★★★ w740 3/3: the doorbell census and its FIRST refusal (row 4) ---"
  grep -a 'doorbells: ' "$Q" 2>/dev/null | tail -1 | cut -c1-200
  grep -ao 'first doorbell refusal \[[^]]*\]' "$Q" 2>/dev/null | tail -1
  echo "W740-CEUTILS-ASSERT=$(grep -ac 'lastCompletedPayload == lastSubmittedPayload' "$BENCH/run_${tag}_dmesg.log" 2>/dev/null)  ⊘ row 5: 0 is the prediction"
  echo "W740-RMINIT-FAILED=$(grep -ac 'RmInitAdapter failed' "$BENCH/run_${tag}_dmesg.log" 2>/dev/null)  ★★★ row 6: 0 is the WIN CONDITION"
  echo "--- ★★★ w739 CUT C 2/3: the BAR1/BAR2 translate tallies (resolved vs REFUSED) ---"
  grep -ao 'BAR2 (translated):.\{0,200\}' "$Q" 2>/dev/null | tail -1
  grep -ao 'BAR1 (translated):.\{0,200\}' "$Q" 2>/dev/null | tail -1
  echo "--- ★★★ w739 CUT C 3/3: FB-IO by role, ON BOTH ARMS (the item-2 pricing) ---"
  n_io=$(grep -ac 'FB-IO trap\[' "$Q" 2>/dev/null)
  echo "W739-FBIO-LINES=${n_io:-0}"
  grep -ao 'FB-IO trap\[.\{0,300\}' "$Q" 2>/dev/null | tail -1 | fold -w 160
  echo "--- the store's FIRST refusal, on its own line (it says its own name once) ---"
  grep -a 'DEVICE-FB ⊘⊘⊘ FIRST HOST-SIDE' "$Q" 2>/dev/null | head -4 | cut -c1-300
  echo "--- ★★★ PREDICTION 1b: the DEVICE-VIEW-PORT census, VERBATIM ---"
  n_pl=$(grep -ac 'DEVICE-VIEW-PORT ' "$Q" 2>/dev/null)
  echo "W736-DEVVIEW-LINES=${n_pl:-0}"
  grep -ao 'DEVICE-VIEW-PORT .\{0,600\}' "$Q" 2>/dev/null | tail -2 | fold -w 160
  echo "--- ★★★ PREDICTION 2: WHERE IT DIED ---"
  echo "[guest NVRM, FIRST lines — not the last: a re-boot attempt masks the first failure]"
  grep -a 'NVRM' "$BENCH/run_${tag}_dmesg.log" 2>/dev/null | head -14 | cut -c1-190
  echo "[the adapter verdict]"
  grep -aE 'RmInitAdapter|kbusVerifyBar2|Cannot (load state into|initialize) the device' \
       "$BENCH/run_${tag}_dmesg.log" 2>/dev/null | head -8 | cut -c1-190
  echo "[nvidia-smi's own exit]"
  grep -a 'SMI_RC=\|MODPROBE_RC=' "$D" 2>/dev/null | tail -3
  echo "--- the client grade (the control's gate; the device arm is expected to have none) ---"
  grep -a 'W392D_GUEST_OUTCOME=' "$D" 2>/dev/null | tail -1 | cut -c1-120
  grep -a 'THREADS ' "$D" 2>/dev/null | tail -1 | cut -c1-90
  grep -a 'MEAN_FALSIFIER' "$D" 2>/dev/null | tail -1 | cut -c1-90
  echo "--- what the device path was asked for, if anything (trap/fill census) ---"
  grep -ao 'TRAPWITNESS[^|]\{0,300\}' "$Q" 2>/dev/null | tail -1
  grep -ao 'arena\[[^]]*\]' "$Q" 2>/dev/null | tail -1
  # ★★★★★ w742 ADDITION, REPORTING ONLY — the publish route's device arm. Cut from the SAME
  # logs, on BOTH arms; the control's zeros are as much a result as the device arm's numbers,
  # because `DEVICE-LEAF` prints unconditionally and `⊘ THE DEVICE ARM NEVER RAN` is the
  # arena arm's CORRECT verdict. ⊘ Every field is cut out of ITS OWN census line.
  echo "--- ★★★★★ w742 ROWS 1/2/11: the publish route's device arm, VERBATIM ---"
  n_dlf=$(grep -ac 'DEVICE-LEAF no_join_needed=' "$Q" 2>/dev/null)
  echo "W742-DEVICELEAF-LINES=${n_dlf:-0}  (0 ⇒ UNMEASURED, not 'the arm did nothing')"
  DL=$(grep -ao 'DEVICE-LEAF no_join_needed=.\{0,600\}' "$Q" 2>/dev/null | tail -1)
  printf '%s\n' "$DL" | fold -w 160
  w742f() { printf '%s' "$DL" | grep -ao "$1" | tail -1; }
  echo "W742-NO-JOIN-NEEDED=$(w742f 'no_join_needed=[0-9]*')   ★ row 2: ≈4431 predicted"
  echo "W742-PLAN-REFUSED=$(w742f 'plan_refused=[0-9]*')"
  echo "W742-SLICES-ARMED=$(w742f 'slices_armed=[0-9]*')   ★ row 11: ≥20 predicted"
  echo "W742-INSTALL-REFUSED=$(grep -ac 'THE INSTALL REFUSED' "$Q" 2>/dev/null)   ★★★ row 1: 0 predicted (w740: 4431)"
  echo "W742-NO-JOIN-LINES=$(grep -ac 'NO JOIN IS NEEDED' "$Q" 2>/dev/null)   (capped at 6 by design)"
  echo "--- ★★★★★ w742 ROW 3: the QUIESCE count — the number the guest actually paid ---"
  echo "W742-QUIESCE=$(grep -ao 'quiesce\[calls=[0-9]* removed=[0-9]*\]' "$Q" 2>/dev/null | tail -1)   ★ row 3: calls ≤ ~150 (w740: 4431)"
  echo "--- ★★★★★ w742 ROWS 4/5/6: THE GRADE. Both BAR1 numbers and BAR2's control ---"
  echo "W742-BAR1-CENSUS=$(grep -ao 'BAR-MIRROR bar1 AT END OF RUN: arm=[a-z]* TRAP_FILLS=[0-9]* premap_fills=[0-9]* distinct_pages=[0-9]* distinct_frames=[0-9]*' "$Q" 2>/dev/null | tail -1)"
  echo "W742-BAR2-CENSUS=$(grep -ao 'BAR-MIRROR bar2 AT END OF RUN: arm=[a-z]* TRAP_FILLS=[0-9]* premap_fills=[0-9]* distinct_pages=[0-9]* distinct_frames=[0-9]*' "$Q" 2>/dev/null | tail -1)"
  echo "W742-BAR1-MISSES=$(grep -ao 'BAR1-PASSTHROUGH arm=[a-z]* misses=[0-9]*' "$Q" 2>/dev/null | tail -1)   ★★★ row 5: misses=0, and THIS is the honest one"
  echo "W742-BAR2-MISSES=$(grep -ao 'BAR2-PASSTHROUGH arm=[a-z]* misses=[0-9]*' "$Q" 2>/dev/null | tail -1)"
  echo "--- ★★★★★ w742 ROW 10: the verb budget the skipped mint returns ---"
  echo "W742-VERBCOST=$(grep -ao 'VERBCOST total=[0-9]*us over [0-9]* plan(s) \[JoinFbLeaf n=[0-9]*[^]]*\]' "$Q" 2>/dev/null | tail -1)   ★ row 10: JoinFbLeaf n ≤ 100 (w740: 2149)"
  echo "--- ★★★★★ w742 ROW 12: the raw client's named fault, direction only ---"
  echo "W742-CPUCEFB=$(grep -ao 'FwdFault::CpuCeFb=[0-9]*' "$Q" 2>/dev/null | tail -1)   ⊘ w740: 63 on the device arm; <63 is the DIRECTION, not a pass"
  echo "W742-CPUCEFB-LINES=$(grep -ac 'CpuCeFb {' "$Q" 2>/dev/null)"
  echo "--- ★★★★★ w742 ROW 7: `named` must not regress ---"
  echo "W742-NAMED=$(printf '%s' "$DFB" | grep -ao 'named=[0-9]*' | tail -1)   ★ row 7: ≥300000 (w740: 426221)"
  echo "--- ⊘ w742: the arena census line — on the device arm it must SAY unmeasured, not print four zeros ---"
  grep -ao 'arena\[[^]]*\]' "$Q" 2>/dev/null | tail -1 | cut -c1-320
  echo "--- host Xid ---"
  echo "HOST_DMESG_XID=$(grep -ac 'Xid' "$BENCH/run_${tag}_hostdmesg.log" 2>/dev/null)"
  grep -a 'Xid' "$BENCH/run_${tag}_hostdmesg.log" 2>/dev/null | head -3 | cut -c1-190
  echo "--- QEMU's own last words (if it refused at realize, the sentence is here) ---"
  grep -aE 'Unsupported|refus|REFUS|⊘' "$Q" 2>/dev/null | tail -12 | cut -c1-260
  echo "######## END arm=$arm ########"
}

# ===================== ARM 1: THE CONTROL (arena, the default) =====================
pkill -f '[q]emu-system-x86'
sleep 3
echo
echo "############ ARM=arena (CONTROL) ############"
unset KAYFABE_FB_STORE
env -u KAYFABE_FB_STORE KAYFABE_DEVICE_VIEW=probe PREFIX="${TAG}arena" SHADOW=on \
    bash scripts/bench/single_store_e6_boot.sh 2>&1 | tee "$BENCH/w736_arena.log"
echo "ARENA_BOOT_RC=$?"
report "${TAG}arena" arena

# ===================== ARM 2: THE DEVICE STORE =====================
pkill -f '[q]emu-system-x86'
sleep 3
echo
echo "############ ARM=device (THE TEST) ############"
KAYFABE_FB_STORE=device KAYFABE_DEVICE_VIEW=probe PREFIX="${TAG}dev" SHADOW=on \
    bash scripts/bench/single_store_e6_boot.sh 2>&1 | tee "$BENCH/w736_dev.log"
echo "DEVICE_BOOT_RC=$?"
report "${TAG}dev" device

echo "=== W736 END $(date -Is) ==="
