#!/usr/bin/env bash
# ★★★★★ IS THE LLM LANE PROVISIONED? — answered WITHOUT taking the GPU.
#
# ⊘⊘ **THE MISTAKE THIS FILE EXISTS TO STOP, AND IT COST THIS SESSION AN HOUR: `/opt/llm`
# IS A PATH INSIDE THE GUEST.** `provision_guest_llm.sh` installs the venv, the torch wheel
# and the model INTO `guest.qcow2`, over a plain QEMU with slirp. Nothing is ever written to
# `/opt/llm` **on the host**. So `ssh vh 'ls /opt/llm'` answers a question nobody asked, and
# its `No such file or directory` reads as *"the lane is unprovisioned"* whether it is or not.
# ⚠ Same class as [[the serial log is NOT where the driver's output is]] — every signal says
# the evidence is there; only looking at the right filesystem shows it is not.
#
# ## ★★★ THREE STATES, THREE EXIT CODES — and "I could not tell" is NOT zero
#   0  PROVISIONED    — verified, and the line says by which of the two witnesses
#   10 UNPROVISIONED  — positively determined absent
#   20 UNKNOWN        — the guest is down AND there is no receipt. ⊘ NOT a result.
# ⊘ A preflight that exits 0 when it could not check is exactly the false green
# [[the_llm_lane_needs_its_own_guest_provisioning]] records: `hook finished: rc=0` over a
# workload that never ran. Refuse to be that.
#
# Read-only. Takes no GPU, boots nothing, and is safe to run while a bench boot is in flight.
set -uo pipefail
BENCH=${BENCH:-/workspace/bench}
RECEIPT="$BENCH/llm_lane.receipt"
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"

say(){ echo "[$(date -Is)] $*"; }
STATE=UNKNOWN; WHY="no witness consulted"; VIA=none
say "LLM_PREFLIGHT_START bench=$BENCH"

# ---- witness 1: the RECEIPT (survives a powered-off guest; this is the GPU-free path) -----
# ⚠ The receipt is a claim about an IMAGE. If guest.qcow2 has been rebuilt since, the receipt
# is stale and says nothing — so compare mtimes rather than trusting its existence.
RECEIPT_STATE=absent
if [ -f "$RECEIPT" ]; then
  if [ -f "$BENCH/guest.qcow2" ] && [ "$BENCH/guest.qcow2" -nt "$RECEIPT" ]; then
    RECEIPT_STATE=stale
    say "⊘ receipt exists but guest.qcow2 is NEWER — the image was rebuilt after provisioning."
    say "   receipt:  $(date -Is -r "$RECEIPT")"
    say "   qcow2:    $(date -Is -r "$BENCH/guest.qcow2")"
    say "   ⇒ treating the receipt as UNMEASURED, not as a pass."
  else
    RECEIPT_STATE=valid
    say "receipt present:"; sed 's/^/    /' "$RECEIPT"
  fi
else
  say "no receipt at $RECEIPT"
fi

# ⊘ A weaker, older signal kept only to explain an absent receipt on a box that WAS
# provisioned before this file existed. Its presence is not a pass on its own.
[ -f "$BENCH/llmprov_serial.log" ] && say "note: llmprov_serial.log exists (a provisioning boot ran at some point)"

# ---- witness 2: the GUEST ITSELF (authoritative, but only if it happens to be up) ---------
GUEST_STATE=down
if $G true >/dev/null 2>&1; then
  GUEST_STATE=up
  say "guest answers on the tap — asking it directly (authoritative)"
  Q=$($G 'test -f /opt/llm/run_llm.py && echo RUNNER=yes || echo RUNNER=no
          test -x /home/ubuntu/llmvenv/bin/python && echo VENV=yes || echo VENV=no
          test -d /opt/llm/hf && echo HFCACHE=yes || echo HFCACHE=no
          /home/ubuntu/llmvenv/bin/python -c "import torch,transformers;print(\"TORCH=\"+torch.__version__)" 2>/dev/null || echo TORCH=no' 2>&1 | tr -d '\r')
  echo "$Q" | sed 's/^/    /'
  R=$(echo "$Q" | sed -n 's/^RUNNER=//p'); V=$(echo "$Q" | sed -n 's/^VENV=//p')
  T=$(echo "$Q" | sed -n 's/^TORCH=//p')
  if [ "$R" = yes ] && [ "$V" = yes ] && [ "$T" != no ] && [ -n "$T" ]; then
    STATE=PROVISIONED; VIA=guest; WHY="runner, venv and torch $T all present in the guest"
  else
    STATE=UNPROVISIONED; VIA=guest
    WHY="guest is up and is MISSING one of runner/venv/torch (RUNNER=$R VENV=$V TORCH=$T)"
  fi
else
  say "guest does not answer on the tap (down, mid-boot, or nvktap0 absent)"
  # ⚠ `nvktap0` does not survive a host reboot and QEMU requires it to pre-exist; and a guest
  # needs ~20-25 s to reach a login prompt. A silent guest is not evidence of anything.
  ip -br addr show nvktap0 2>&1 | sed 's/^/    tap: /'
  case "$RECEIPT_STATE" in
    valid) STATE=PROVISIONED;   VIA=receipt; WHY="receipt is present and newer than guest.qcow2" ;;
    *)     STATE=UNKNOWN;       VIA=none
           WHY="guest is unreachable and the receipt is $RECEIPT_STATE — nothing was measured" ;;
  esac
fi

echo ""
echo "=== ★★★★★ THE VERDICT, stated once"
case "$STATE" in
  PROVISIONED)
    echo "    LLM_LANE=PROVISIONED via=$VIA"
    echo "    $WHY"
    echo "    ⇒ the lane can be RUN: boot with POST_CAPTURE_HOOK=scripts/bench/w392_llm.sh"
    RC=0 ;;
  UNPROVISIONED)
    echo "    LLM_LANE=UNPROVISIONED via=$VIA"
    echo "    $WHY"
    echo "    ⇒ run scripts/bench/provision_guest_llm.sh FIRST (~7-20 min, needs the bench idle)."
    echo "    ⊘ Until then any LLM verdict is (E) UNMEASURED, never a kayfabe result."
    RC=10 ;;
  *)
    echo "    LLM_LANE=UNKNOWN"
    echo "    $WHY"
    echo "    ⊘ THIS IS NOT 'unprovisioned' AND NOT 'ok'. Bring the guest up, or re-run after"
    echo "      the current boot finishes, and ask again."
    RC=20 ;;
esac
say "LLM_PREFLIGHT_DONE state=$STATE rc=$RC"
exit $RC
