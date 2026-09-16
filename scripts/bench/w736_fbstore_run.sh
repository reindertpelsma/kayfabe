#!/usr/bin/env bash
# ★★★★★ w745 — REUSED, AND THE ONLY STRUCTURAL CHANGE SINCE w736: a THIRD ARM.
#   Constraint 26's ownership split is a second variable (`KAYFABE_VAS_OWNER`), so the
#   two-arm shape can no longer separate "the store" from "who maps into the VA space".
#   The arms are now:
#     1  arena    + isolate     — THE CONTROL. Must stay `(P)` 8/8. Byte-identical to w743.
#     2  device   + isolate     — w743's device arm, REPRODUCED on this binary. It is what
#                                 says a change in arm 3 is the SPLIT and not the rebuild.
#     3  device   + scratchpad  — THE TEST.
#   ⊘ Arm 2 is not optional and is not padding: without it, arm 3's numbers are being
#   compared against another day's boot on another revision, which is the comparison
#   `a_rulings_date_is_part_of_the_citation` exists to refuse.
#   ⚠ `env -u` clears BOTH variables on the arms that must not see them — a variable left
#   set from a previous arm is how a "control" comes to run the thing it controls for.
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
# ★★★ w745 — THE SAME GATE, FOR THIS CHANGE. `STORE-MAP` prints UNCONDITIONALLY on BOTH
# arms at teardown (`⊘ NOT BUILT` on the control), so a boot without the string is an OLDER
# BINARY and every w745 row would be graded against absence. ⚠ The most dangerous of those
# rows is `adopts=0 maps=0`, which on a stale binary is indistinguishable from "the split
# ran and had nothing to map".
n_w745=$(strings "$Q_BIN" 2>/dev/null | grep -c 'STORE-MAP ')
n_w745b=$(strings "$Q_BIN" 2>/dev/null | grep -c 'RING-NOT-A-SLICE')
n_w745c=$(strings "$Q_BIN" 2>/dev/null | grep -c 'DEVICE-LEAF-SPLIT')
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
echo "W745-CONTENT: store_map=$n_w745 ring_not_a_slice=$n_w745b split_census=$n_w745c (any 0 ⇒ the binary predates w745)"
if [ "$n_w745" -eq 0 ] || [ "$n_w745b" -eq 0 ] || [ "$n_w745c" -eq 0 ]; then echo "⊘ the binary predates w745 — its rows cannot be graded. STOP."; exit 10; fi
# ★★★★★ w746 ADDITION — CONTENT GATE. Every w746 row below is about a string this revision
# introduced; a binary that predates it prints zeros that read exactly like measured zeros.
# ⊘ The bench served a binary built from `862c7c2` for weeks; this is the check that makes
# that impossible for THIS increment's rows rather than a thing to remember.
n_w746=$(strings "$Q_BIN" 2>/dev/null | grep -c 'HANDOVER-ASSERTS')
n_w746b=$(strings "$Q_BIN" 2>/dev/null | grep -c 'THE HAND-OVER WAS REFUSED')
echo "W746-CONTENT: handover_asserts=$n_w746 refusal_line=$n_w746b (any 0 ⇒ the binary predates w746)"
if [ "$n_w746" -eq 0 ] || [ "$n_w746b" -eq 0 ]; then echo "⊘ the binary predates w746 — its rows cannot be graded. STOP."; exit 11; fi
# ★★★★★ w752 ADDITION — CONTENT GATE. `PRAMIN-INPLACE` prints UNCONDITIONALLY whenever the
# device-view port exists, on BOTH device arms, so a boot without the string is an OLDER
# BINARY and every w752 row would be graded against absence. ⚠ The most dangerous of those
# rows is `declined_on_vcpu=0`, which on a stale binary is indistinguishable from "cut P2 ran
# and the decline was never on the path" — the exact `a_refusal_counter_read_as_absent_demand`
# shape this increment's own census line was written to refuse.
n_w752=$(strings "$Q_BIN" 2>/dev/null | grep -c 'PRAMIN-INPLACE AT')
echo "W752-CONTENT: pramin_inplace=$n_w752 (0 ⇒ the binary predates w752 — its rows cannot be graded)"
if [ "$n_w752" -eq 0 ]; then echo "⊘ the binary predates w752 — its rows cannot be graded. STOP."; exit 12; fi
# ★★★★★ w754 ADDITION — CONTENT GATE. `GSP-HEAD` and `RING-ADOPT` print UNCONDITIONALLY at
# teardown on BOTH arms, so a boot without them is an OLDER BINARY and every w754 row would be
# graded against absence. ⚠ The most dangerous of those rows is `RING-ADOPT on_vcpu=0`, which
# on a stale binary is indistinguishable from *"the settlement really did leave the vCPU"* —
# the whole claim of this rung, read off a string that was never printed.
n_w754=$(strings "$Q_BIN" 2>/dev/null | grep -c 'RING-ADOPT ran=')
n_w754b=$(strings "$Q_BIN" 2>/dev/null | grep -c 'GSP-HEAD posted=')
echo "W754-CONTENT: ring_adopt=$n_w754 gsp_head=$n_w754b (either 0 ⇒ the binary predates w754)"
if [ "$n_w754" -eq 0 ] || [ "$n_w754b" -eq 0 ]; then echo "⊘ the binary predates w754 — its rows cannot be graded. STOP."; exit 13; fi

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
  # ★★★★★ w745 ADDITION, REPORTING ONLY — constraint 26's own census, cut from the SAME
  # logs, on ALL THREE arms. ⊘ The control's `⊘ NOT BUILT` is as much a result as the test
  # arm's numbers: it is what says the split was absent rather than present-and-silent.
  echo "--- ★★★★★ w745 ROWS 1/2: the STORE-MAP census, VERBATIM ---"
  n_sm=$(grep -ac 'STORE-MAP ' "$Q" 2>/dev/null)
  echo "W745-STOREMAP-LINES=${n_sm:-0}  (0 ⇒ UNMEASURED, not 'the split did nothing')"
  SM=$(grep -ao 'STORE-MAP iso=.\{0,700\}' "$Q" 2>/dev/null | tail -1)
  printf '%s\n' "$SM" | fold -w 160
  w745f() { printf '%s' "$SM" | grep -ao "$1" | tail -1; }
  echo "W745-ADOPTS=$(w745f 'adopts=[0-9]*')        ★ row 1: ≥1 on the scratchpad arm"
  echo "W745-ADOPT-REFUSED=$(w745f 'adopt_refused=[0-9]*')"
  echo "W745-MAPS=$(w745f ' maps=[0-9]*')           ★★★ row 2: ≥1 is the whole increment"
  echo "W745-MAP-REFUSED=$(w745f 'map_refused=[0-9]*')"
  echo "W745-OUTSTANDING=$(w745f 'outstanding=[0-9]*')"
  echo "W745-ASSERTED=$(w745f 'asserted=[0-9]*')    ⊘ 0 with maps>0 means births never reached the oracle"
  echo "W745-ASSERT-REFUSED=$(w745f 'assert_refused=[0-9]*')"
  echo "W745-SM-FIRST=$(printf '%s' "$SM" | grep -ao 'first_refusal=\[[^]]*\]' | tail -1)"
  echo "--- w745: the realize-time arm line (absent ⇒ the gate never ran) ---"
  grep -a 'STORE-MAP AT REALIZE' "$Q" 2>/dev/null | head -2 | cut -c1-320
  echo "--- ★★★★★ w745 ROWS 3/4: the PUBLISH-side census, VERBATIM, on both arms ---"
  n_dls=$(grep -ac 'DEVICE-LEAF-SPLIT' "$Q" 2>/dev/null)
  echo "W745-SPLITCENSUS-LINES=${n_dls:-0}  (0 ⇒ UNMEASURED, not 'the arm did nothing')"
  grep -ao 'DEVICE-LEAF-SPLIT .\{0,400\}' "$Q" 2>/dev/null | tail -1 | fold -w 160
  echo "W745-DECLINED-ON-VCPU=$(grep -ao 'declined_on_vcpu=[0-9]*' "$Q" 2>/dev/null | tail -1)   ⊘ read BEFORE any refusal count"
  echo "--- ★★★★★ w745 ROW 3: THE HAND-OVER, and the per-proc isolate's own refusal if any ---"
  echo "W745-HANDOVERS=$(grep -ac 'CONSTRAINT-26 HAND-OVER' "$Q" 2>/dev/null)"
  grep -a 'CONSTRAINT-26 HAND-OVER' "$Q" 2>/dev/null | head -3 | cut -c1-300
  grep -a 'CONSTRAINT 26: the per-proc isolate would not hand' "$Q" 2>/dev/null | head -2 | cut -c1-300
  echo "--- ★★★★★ w745 ROW 4: THE SLICE BINDINGS — the line conjunct (6) needs ---"
  echo "W745-SLICE-BOUND=$(grep -ac 'CONSTRAINT 26: the SCRATCHPAD mapped a slice' "$Q" 2>/dev/null)"
  grep -a 'CONSTRAINT 26: the SCRATCHPAD mapped a slice' "$Q" 2>/dev/null | head -2 | cut -c1-320
  echo "--- ★★★★★ w745 ROW 5: THE FOUR FROZEN NUMBERS. Any movement is the finding ---"
  echo "W745-ADOPTWHY-6=$(grep -ac 'ADOPT-WHY.*(6) the binding EXISTS but carries NO HOST OBJECT' "$Q" 2>/dev/null)   ⊘ w740/w742/w743: 17, 17, 17"
  echo "W745-RING-NOT-A-SLICE=$(grep -ac 'RING-NOT-A-SLICE' "$Q" 2>/dev/null)   ★ NEW refusal; non-zero here with ADOPTWHY-6=0 is the wall having MOVED"
  echo "W745-BIRTH-REFUSED=$(grep -ac 'BIRTH-AT-ALLOC.*REFUSED' "$Q" 2>/dev/null)   ⊘ w740/w742/w743: 11, 11, 11"
  echo "W745-BIRTH-ADOPTING=$(grep -ac 'BIRTH-AT-ALLOC.*ADOPTING at creation' "$Q" 2>/dev/null)   ★★★ 0 on all three prior boots"
  echo "W745-DOORBELL-BIRTH=$(grep -ao 'FwdFault::PassthroughDoorbellBirth=[0-9]*' "$Q" 2>/dev/null | tail -1)   ⊘ w740/w742/w743: 19, 19, 19"
  echo "W745-FOREIGN-HANDLE=$(grep -ac 'ForeignHandle' "$Q" 2>/dev/null)"
  # ⊘⊘⊘ **w746 — THIS GREP WAS BLIND TO ITS OWN REFUSAL, AND THE BOOT PROVED IT.**
  # `[measured w746, split arm]` the refusal fired **10 times** and this row reported **0**,
  # because `RmError::Other` is printed through `Debug` in DECIMAL — `Other(19266)` — and the
  # pattern asked for `0x4b42` and the constant's NAME, neither of which ever appears in the
  # log. ⇒ a counter that cannot see the thing it counts, inside the increment whose subject
  # is exactly that. The decimal spelling is now first.
  echo "W745-BARE-SPACE-REFUSED=$(grep -ac 'Other(19266)\|0x4b42\|MAP_THROUGH_A_BARE_SPACE' "$Q" 2>/dev/null)   ⊘ 19266 = 0x4B42; the Debug print is DECIMAL"
  echo "W746-SCRATCHPAD-BIRTH-REFUSED=$(grep -ac 'Other(19270)\|SCRATCHPAD_BIRTH_IN_A_HANDED_SPACE' "$Q" 2>/dev/null)   ⊘ 19270 = 0x4B46 (constraint 30)"
  echo "W746-RING-HANDLE-RM-DEC=$(grep -ac 'Other(19269)\|RING_HANDLE_REACHED_RM' "$Q" 2>/dev/null)   ⊘ 19269 = 0x4B45 (constraint 29)"
  echo "W746-HANDOVER-NON-BARE=$(grep -ac 'Other(19267)\|HANDOVER_OF_A_NON_BARE_SPACE' "$Q" 2>/dev/null)   ⊘ 19267 = 0x4B43"
  echo "W746-ADOPT-NOT-SCRATCHPAD=$(grep -ac 'Other(19265)\|ADOPT_NOT_THE_SCRATCHPAD' "$Q" 2>/dev/null)   ⊘ 19265 = 0x4B41"
  echo "--- ★★★★★ w746 ROWS: THE HAND-OVER'S ENSURE PATH AND ITS THREE ASSERTS ---"
  # ⊘ `asked` beside every fire. w745 read `RING-NOT-A-SLICE=0` and `FOREIGN-HANDLE=0` as
  # passes on a boot where `asserted=0`; a gate's zero is a pass only if the gate RAN.
  echo "W746-ASSERTS-LINE=$(grep -ac 'HANDOVER-ASSERTS' "$Q" 2>/dev/null)  (0 ⇒ UNMEASURED)"
  grep -ao 'HANDOVER-ASSERTS .\{0,400\}' "$Q" 2>/dev/null | tail -1 | fold -w 160
  echo "W746-HANDOVER-MINTED=$(grep -ac 'CONSTRAINT-26 HAND-OVER' "$Q" 2>/dev/null)   ★★★ row A: ≥1. w745: 0, and the cause was that host_vas was never minted"
  echo "W746-HANDOVER-REFUSED-LINES=$(grep -ac 'THE HAND-OVER WAS REFUSED' "$Q" 2>/dev/null)"
  grep -a 'THE HAND-OVER WAS REFUSED' "$Q" 2>/dev/null | head -3 | cut -c1-320
  echo "W746-ROUTE-DISAGREES=$(grep -ac 'HandoverRouteDisagrees' "$Q" 2>/dev/null)   ⊘ ANY non-zero is a DEFECT, never a transient"
  echo "W746-C30-REFUSED=$(grep -ac 'REFUSED CONSTRAINT 30' "$Q" 2>/dev/null)   ⊘ constraint 30: a space that is not this proc's own isolate's"
  echo "W746-C30-BIRTH-REFUSED=$(grep -ac 'SCRATCHPAD_BIRTH_IN_A_HANDED_SPACE' "$Q" 2>/dev/null)   ⊘ constraint 30: the scratchpad birthing in an adopted space"
  echo "W746-RING-HANDLE-REACHED-RM=$(grep -ac 'RING_HANDLE_REACHED_RM' "$Q" 2>/dev/null)   ⊘ constraint 29: the deleted \`AdoptedGuestRing::memory\`'s premise breaking"
  echo "--- ★★★★★ w745 ROW 6: CONSTRAINT 27's barrier, on every arm ---"
  echo "W745-WITHHELD-UNMAPS=$(grep -ac 'WITHHELD-UNMAPS' "$Q" 2>/dev/null)"
  echo "W745-MMUINVAL=$(grep -ao 'MMUINVAL armed=.\{0,400\}' "$Q" 2>/dev/null | tail -1)"
  echo "W745-REFRESH-LINE=$(grep -ao 'MMUINVAL-REFRESH #[0-9]* seq=[0-9]* armed=[0-9]* refresh_ms=[0-9.]* unmaps_outstanding=[0-9]* drain_trips=[0-9]*' "$Q" 2>/dev/null | tail -1)"
  echo "--- ★★★★★ w745 ROW 7: CONSTRAINT 25's open violation — the number the brief asks for ---"
  # ⊘⊘ w750 FIX, REPORTING ONLY — THIS ROW PRINTED EMPTY ON EVERY ARM, AND AN EMPTY ROW
  #    READS AS A ZERO. The pattern demanded a `worst_trap=<n>us` field the line does not
  #    carry: the census emits `VCPU-BLOCKING total=197 doors=9 [40 × receiving a
  #    descriptor across the isolate boundary, …]`. `[measured w750, 2026-09-16, all three
  #    arms]` the number was in `$Q` the whole time and this row said nothing about it —
  #    the exact shape of `a_check_that_reports_is_not_a_check_that_gates`, on the one row
  #    whose comment above says it is *"the number the brief asks for"*.
  #    ⇒ the tail is taken verbatim instead of being matched, so a future field change
  #      cannot silence it again.
  echo "W745-VCPU-BLOCKING=$(grep -ao 'VCPU-BLOCKING total=[0-9]* doors=[0-9]*' "$Q" 2>/dev/null | tail -1)"
  echo "W745-VCPU-BLOCKING-DOORS=$(grep -aohE 'VCPU-BLOCKING \(door #[0-9]+\)' "$Q" 2>/dev/null | sort -u | wc -l)"
  # ⊘⊘ MERGE NOTE (w750 x w752): w752's copy of the W745 row still carried the broken
  #    `worst_trap=[0-9]*us` pattern w750 had just deleted. Taking w752 wholesale would have
  #    SILENTLY REINTRODUCED an empty row that reads as a zero. Kept w750's grep, took w752's
  #    additions -- two fixes to one line, and only one of them survives a blind `--theirs`.
  # ★★★★★ w752 ADDITION, REPORTING ONLY — the door census IN FULL, and PRAMIN's own numbers.
  # ⊘ The w745 grep above cuts only the HEADER. `total=` and `doors=` cannot say WHICH doors
  # survived, and w752's whole claim is about four named ones (4, 6, 7, 8) going to zero while
  # three named ones (1, 2, 3) plus the sanctioned MAP_FIXED (5) stay. A header-only capture
  # would grade "197 -> 102" as a pass even if the residual were the wrong doors entirely.
  echo "--- ★★★★★ w752 ROW 1: THE DOOR CENSUS IN FULL (every [n x door] row) ---"
  echo "W752-VCPU-LINES=$(grep -ac 'VCPU-BLOCKING ' "$Q" 2>/dev/null)  (0 ⇒ UNMEASURED)"
  VB=$(grep -ao 'VCPU-BLOCKING .\{0,1200\}' "$Q" 2>/dev/null | tail -1)
  printf '%s\n' "$VB" | fold -w 160
  d() { printf '%s' "$VB" | grep -ao "$1" | tail -1; }
  echo "W752-DOOR-WINDOW-MMAP=$(d '\[[0-9]* × mmap (creating a guest-physical window)\]')   ⊘ door 4 — predicted EXACTLY 1 (the one-time install)"
  echo "W752-DOOR-SLOT-INSTALL=$(d '\[[0-9]* × KVM_SET_USER_MEMORY_REGION (installing a memslot)\]')   ⊘ door 6 — predicted EXACTLY 1"
  echo "W752-DOOR-SLOT-DROP=$(d '\[[0-9]* × KVM_SET_USER_MEMORY_REGION (dropping a memslot)\]')   ⊘ door 7 — predicted ABSENT"
  echo "W752-DOOR-MUNMAP=$(d '\[[0-9]* × munmap (dropping a guest-physical window)\]')   ⊘ door 8 — predicted ABSENT"
  echo "W752-DOOR-RELEASE=$(d '\[[0-9]* × releasing a host device view\]')   ⊘ door 9 — predicted ABSENT (moved to the worker tick)"
  echo "W752-DOOR-MAPFIXED=$(d '\[[0-9]* × mmap MAP_FIXED (placing an armed device node)\]')   ★ door 5 — STAYS, one per move"
  echo "W752-DOOR-EXPORT=$(d '\[[0-9]* × exporting a host device view to the VMM\]')   ★ door 1 — STAYS"
  echo "W752-DOOR-RECV=$(d '\[[0-9]* × receiving a descriptor across the isolate boundary\]')   ★ door 2 — STAYS, TWO per arm"
  echo "W752-DOOR-CLASSIFY=$(d '\[[0-9]* × classifying a received descriptor\]')   ★ door 3 — STAYS"
  echo "--- ★★★★★ w752 ROW 2: PRAMIN's OWN census — the move cost and the cut's counters ---"
  echo "W752-PRAMIN-SLOT=$(grep -ao 'PRAMIN-SLOT AT [A-Z]*: moves=[0-9]* skipped=[0-9]*' "$Q" 2>/dev/null | tail -1)"
  echo "W752-MOVE-NS=$(grep -ao 'move_ns\[worst=[0-9]* mean=[0-9]*\]' "$Q" 2>/dev/null | tail -1)   ⊘ w742 device arm: worst=44426000 mean=7248000 (ns)"
  echo "W752-INPLACE-LINES=$(grep -ac 'PRAMIN-INPLACE AT' "$Q" 2>/dev/null)  (0 ⇒ UNMEASURED — printed whenever the port exists, so absence is an old binary)"
  PI=$(grep -ao 'PRAMIN-INPLACE AT .\{0,600\}' "$Q" 2>/dev/null | tail -1)
  printf '%s\n' "$PI" | fold -w 160
  h() { printf '%s' "$PI" | grep -ao "$1" | tail -1; }
  echo "W752-INPLACE=$(h 'inplace=[0-9]*')   ★ the moves that were ONE MAP_FIXED"
  echo "W752-INPLACE-REFUSED=$(h 'inplace_refused=[0-9]*')   ⊘ any non-zero tore the slot down; PRAMIN then traps"
  # ⊘ `inplace_refused`, not `refused`: `early_release_refused=` also ends in `refused=` and a
  # loose grep with `tail -1` would silently report THAT number instead. The two mean opposite
  # things — a torn-down aperture vs a held one — and one grep cannot serve both.
  echo "W752-STARTED=$(h 'started=[0-9]*')"
  echo "W752-LANDED=$(h 'landed=[0-9]*')   ⊘ started != landed ⇒ a re-point did not place"
  echo "W752-RELEASED=$(h 'released=[0-9]*')   ★ cut P2: released BY THE WORKER, not inside a trap"
  echo "W752-HELD=$(h 'held=[0-9]*')"
  echo "W752-DECLINED-ON-VCPU=$(h 'declined_on_vcpu=[0-9]*')   ⊘⊘ MEASURED w752: 0, and cut P2 STILL WORKS — cut P1 deleted the only vCPU-side caller, so read W752-DOOR-RELEASE (must be ABSENT) beside W752-RELEASED (must equal W752-INPLACE) instead"
  echo "W752-EARLY-REFUSED=$(h 'early_release_refused=[0-9]*')   ⊘⊘ non-zero = the restated w735 barrier FIRED (an aperture leak, the safe direction)"
  echo "W752-PORT-OUTSTANDING=$(h 'port_outstanding=[0-9]*')"
  echo "--- ★★★★★ w752 ROW 3: the SECOND constraint-4 violator the door census CANNOT see ---"
  echo "⊘ Predicted, not hoped: worst_trap does NOT go sub-ms. The control arm's 22 311 us at"
  echo "  bar0+0x110c00 (NV_PGSP_QUEUE_HEAD) passes through no assert_lock_free door at all."
  echo "W752-TRAPWITNESS=$(grep -ao 'TRAPWITNESS off_trap_claims=.\{0,220\}' "$Q" 2>/dev/null | tail -1)"
  echo "W752-TRAP-CPU=$(grep -ao 'TRAP-CPU .\{0,200\}' "$Q" 2>/dev/null | tail -1)"
  echo "--- ★ w752: any IN-PLACE RE-POINT REFUSED line, verbatim ---"
  grep -a 'IN-PLACE RE-POINT REFUSED' "$Q" 2>/dev/null | head -3 | cut -c1-320
  echo "--- host Xid ---"
  echo "HOST_DMESG_XID=$(grep -ac 'Xid' "$BENCH/run_${tag}_hostdmesg.log" 2>/dev/null)"
  grep -a 'Xid' "$BENCH/run_${tag}_hostdmesg.log" 2>/dev/null | head -3 | cut -c1-190
  # ★ w753 ADDITION, REPORTING ONLY — constraint 32's rows. No arm, threshold or boot step.
  echo "--- w753: route K (constraint 32) ---"
  grep -a 'ROUTE-K AT REALIZE' "$Q" 2>/dev/null | tail -1 | cut -c1-300
  grep -a 'STORE-MAP-K ' "$Q" 2>/dev/null | tail -1 | cut -c1-300
  echo -n "    CONSTRAINT-32 BIRTH-CLIENT MINTED  = "; grep -ac 'BIRTH-CLIENT MINTED' "$Q" 2>/dev/null
  echo -n "    CONSTRAINT-32 BIRTH-CLIENT ADOPTED = "; grep -ac 'BIRTH-CLIENT ADOPTED' "$Q" 2>/dev/null
  echo -n "    CONSTRAINT-32 BIRTH-CLIENT HANDED  = "; grep -ac 'BIRTH-CLIENT HANDED' "$Q" 2>/dev/null
  echo -n "    CONSTRAINT-32 ADOPT-IN-B           = "; grep -ac 'CONSTRAINT-32 ADOPT-IN-B' "$Q" 2>/dev/null
  echo -n "    CONSTRAINT 32 REFUSED (any)        = "; grep -ac 'CONSTRAINT 32 REFUSED' "$Q" 2>/dev/null
  echo "    ⊘ MINTED>0 with ADOPTED=0 means the fd crossed and the far side refused — read"
  echo "      the REFUSED line. MINTED=0 on the k arm means the publish path never reached"
  echo "      the mint, which is NOT evidence route K failed."
  echo "--- QEMU's own last words (if it refused at realize, the sentence is here) ---"
  grep -aE 'Unsupported|refus|REFUS|⊘' "$Q" 2>/dev/null | tail -12 | cut -c1-260
  # ★★★★★ w754 ADDITION, REPORTING ONLY — constraint 4's actual gate and the two censuses that
  # say whether the mechanism behind it ran. ⊘ Nothing here changes an arm, a threshold or a
  # boot step; it cuts three lines out of the SAME logs. Every field is taken out of ITS OWN
  # census line, never grepped loose (w731/w732 both lifted a field off the wrong subsystem).
  echo "--- ★★★★★ w754 ROW 1: THE GRADE — worst_trap, its SITE, and its CPU half ---"
  echo "W754-TRAPWITNESS=$(grep -ao 'TRAPWITNESS off_trap_claims=.\{0,220\}' "$Q" 2>/dev/null | tail -1)"
  echo "W754-TRAPCPU=$(grep -ao 'TRAP-CPU n=.\{0,260\}' "$Q" 2>/dev/null | tail -1)"
  echo "--- ★★★★★ w754 ROW 2: SLOW-SITES — the DISTRIBUTION, because a maximum is one event ---"
  echo "W754-SLOWSITES-LINES=$(grep -ac 'SLOW-SITES bar0' "$Q" 2>/dev/null)  (0 ⇒ UNMEASURED, and note the"
  echo "                        REALIZE prose also contains the string 'SLOW-SITES' — anchored on 'bar0')"
  grep -ao 'SLOW-SITES bar0.\{0,400\}' "$Q" 2>/dev/null | tail -1 | fold -w 160
  echo "--- ★★★★★ w754 ROW 3: did the moved body RUN, and on which thread? ---"
  echo "W754-RINGADOPT-LINES=$(grep -ac 'RING-ADOPT ran=' "$Q" 2>/dev/null)  (0 ⇒ UNMEASURED, not 'it never ran')"
  RA=$(grep -ao 'RING-ADOPT ran=.\{0,500\}' "$Q" 2>/dev/null | tail -1)
  printf '%s\n' "$RA" | fold -w 160
  ra() { printf '%s' "$RA" | grep -ao "$1" | tail -1; }
  echo "W754-ADOPT-RAN=$(ra 'ran=[0-9]*')"
  echo "W754-ADOPT-OFF-VCPU=$(ra 'off_vcpu=[0-9]*')   ★★★ the device arm's GATE: must be > 0"
  echo "W754-ADOPT-ON-VCPU=$(ra 'on_vcpu=[0-9]*')     ⊘⊘ > 0 on a deferring arm = constraint 4 violated"
  echo "W754-ADOPT-NOTHING=$(ra 'nothing_pending=[0-9]*')"
  echo "W754-GRJOIN-BLOCKS=$(grep -ac 'GR-RING-JOIN arm=' "$Q" 2>/dev/null)   ⊘ w752 device arm: 61"
  echo "--- ★★★★★ w754 ROW 4: constraint 6 — the queue-head post and its fold ---"
  echo "W754-GSPHEAD-LINES=$(grep -ac 'GSP-HEAD posted=' "$Q" 2>/dev/null)  (0 ⇒ UNMEASURED)"
  GH=$(grep -ao 'GSP-HEAD posted=.\{0,400\}' "$Q" 2>/dev/null | tail -1)
  printf '%s\n' "$GH" | fold -w 160
  gh() { printf '%s' "$GH" | grep -ao "$1" | tail -1; }
  echo "W754-HEAD-POSTED=$(gh 'posted=[0-9]*')"
  echo "W754-HEAD-FOLDED=$(gh 'folded=[0-9]*')   ⊘⊘⊘ posted>0 with folded=0 is a PARKED GUEST"
  echo "--- ★ w754: the lock census, which is how the 25 ms was shown NOT to be the plane lock ---"
  echo "W754-LOCKCOST=$(grep -ao 'LOCKCOST(>1000us) \[rank0[^]]*\]' "$Q" 2>/dev/null | tail -1)"
  echo "######## END arm=$arm ########"
}

# ===================== ARM 1: THE CONTROL (arena, the default) =====================
pkill -f '[q]emu-system-x86'
sleep 3
echo
echo "############ ARM=arena (CONTROL) ############"
unset KAYFABE_FB_STORE
unset KAYFABE_VAS_OWNER
env -u KAYFABE_FB_STORE -u KAYFABE_VAS_OWNER KAYFABE_DEVICE_VIEW=probe PREFIX="${TAG}arena" SHADOW=on \
    bash scripts/bench/single_store_e6_boot.sh 2>&1 | tee "$BENCH/w736_arena.log"
echo "ARENA_BOOT_RC=$?"
report "${TAG}arena" arena

# ===================== ARM 2: THE DEVICE STORE, OLD OWNERSHIP =====================
# ⊘ w745: this arm is w743's device arm REPRODUCED ON THIS BINARY. It is what makes arm 3's
# numbers attributable to the ownership split rather than to everything else that changed.
pkill -f '[q]emu-system-x86'
sleep 3
echo
echo "############ ARM=device (w743 REPRODUCTION) ############"
env -u KAYFABE_VAS_OWNER KAYFABE_FB_STORE=device KAYFABE_DEVICE_VIEW=probe \
    PREFIX="${TAG}dev" SHADOW=on \
    bash scripts/bench/single_store_e6_boot.sh 2>&1 | tee "$BENCH/w736_dev.log"
echo "DEVICE_BOOT_RC=$?"
report "${TAG}dev" device

# ===================== ARM 3: THE TEST =====================
# ★★★★★ w753 — ARM 3's VALUE IS NOW A PARAMETER, and the harness is NOT forked.
#   `ARM3_VAS_OWNER` selects which scratchpad-ownership arm is under test:
#     scratchpad  (the default) — constraint 26, w745's split. UNCHANGED.
#     k                          — constraint 32, route K: the split PLUS a per-proc birth
#                                  client, so the VA-space dup is same-ProcessID.
#   ⊘ A parameter and not a second script, for the reason three comments at the top of this
#     file already give: a second harness for the same job is what they exist to prevent.
#     Arms 1 and 2 are byte-identical either way, which is the whole point of running them.
#   ⚠ It is PRINTED, because a boot that does not say which arm it ran is uninterpretable —
#     and the two arms differ in exactly one decision.
ARM3_VAS_OWNER="${ARM3_VAS_OWNER:-scratchpad}"
case "$ARM3_VAS_OWNER" in
  scratchpad|k) : ;;
  *) echo "ARM3_VAS_OWNER must be 'scratchpad' or 'k', got '$ARM3_VAS_OWNER'"; exit 2 ;;
esac
pkill -f '[q]emu-system-x86'
sleep 3
echo
# ⊘⊘ **w754 — WHICH THIRD ARM, and the default is UNCHANGED.** `ARM3=split` (the default) is
# w745's constraint-26 arm, byte-identical to every previous run of this harness, so a lane
# that reuses this file is unaffected. `ARM3=inline` swaps it for w754's KNOWN-POSITIVE: the
# SAME binary and the SAME device arm with `KAYFABE_MATERIALIZE_INLINE=1`, which forces the
# ring adopt and its page-table settlement back ONTO the vCPU.
# ★ That arm is not padding — it is the A/B that makes arm 2's number attributable. One env
# var, opposite result, same binary: `RING-ADOPT on_vcpu>0` and `worst_trap` back at
# ~25 ms at `bar0+0x110c00`. Without it, `on_vcpu=0` on arm 2 is a counter nobody has shown
# can move, which is this campaign's most-repeated failure class.
if [ "${ARM3:-split}" = "inline" ]; then
echo "############ ARM=inline (w754 KNOWN-POSITIVE — the settlement forced back on the vCPU) ############"
env -u KAYFABE_VAS_OWNER KAYFABE_FB_STORE=device KAYFABE_DEVICE_VIEW=probe \
    KAYFABE_MATERIALIZE_INLINE=1 \
    PREFIX="${TAG}inline" SHADOW=on \
    bash scripts/bench/single_store_e6_boot.sh 2>&1 | tee "$BENCH/w754_inline.log"
echo "INLINE_BOOT_RC=$?"
report "${TAG}inline" inline
else
echo "############ ARM=split (THE TEST — constraint 26) ############"
KAYFABE_VAS_OWNER=scratchpad KAYFABE_FB_STORE=device KAYFABE_DEVICE_VIEW=probe \
else
# ⊘⊘ MERGE (w753 x w754): w753 parameterised THIS arm's VAS owner while w754 wrapped it in an
#    inline known-positive branch. Neither side is optional -- w754's arm is the A/B that makes
#    `on_vcpu=0` attributable, and w753's variable is what selects route K. Dropping either
#    would leave a counter nobody has shown can move.
echo "############ ARM=split (THE TEST — KAYFABE_VAS_OWNER=$ARM3_VAS_OWNER) ############"
echo "W753_ARM3_VAS_OWNER=$ARM3_VAS_OWNER"
KAYFABE_VAS_OWNER="$ARM3_VAS_OWNER" KAYFABE_FB_STORE=device KAYFABE_DEVICE_VIEW=probe \
    PREFIX="${TAG}split" SHADOW=on \
    bash scripts/bench/single_store_e6_boot.sh 2>&1 | tee "$BENCH/w745_split.log"
echo "SPLIT_BOOT_RC=$?"
report "${TAG}split" split
fi

echo "=== W736 END $(date -Is) ==="
