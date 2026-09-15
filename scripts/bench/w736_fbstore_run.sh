#!/usr/bin/env bash
# ★★★★★ w736 — THE CUT-A BOOT. Tests `SINGLE_STORE_PLAN.md`'s w735 pre-registered prediction.
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
echo "W736-CONTENT: walk_shadow=$n_ws cuda_image=$n_img fb_store=$n_fs device_fb=$n_dfb"
if [ "$n_img" -eq 0 ]; then echo "⊘ no cuda-scratchpad in the binary — the port cannot arm. STOP."; exit 5; fi
if [ "$n_fs" -eq 0 ] || [ "$n_dfb" -eq 0 ]; then echo "⊘ the binary predates cut A. STOP."; exit 4; fi

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
