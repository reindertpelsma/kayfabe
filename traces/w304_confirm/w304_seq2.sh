#!/usr/bin/env bash
# w304 part 2 — the JOINT boot (all five deleted, nothing set) and the (E) KNOWN-POSITIVE.
set -uo pipefail
R=/workspace/kayfabe_w304
S=/workspace/w304_summary.txt
run() {
  local label=$1; shift
  echo "############ w304 ARM $label [$*] $(date -Is) ############" | tee -a "$S"
  "$R/scripts/bench/w304_confirm.sh" "$label" "$@"
  local rc=$?
  echo "############ w304 ARM $label DONE rc=$rc $(date -Is) ############" | tee -a "$S"
}
# ★ THE JOINT BOOT. Confirming five arms individually does not confirm them jointly; this
#   boot is what closes that gap. No overrides: the five are gone BY CONSTRUCTION now.
run joint
# ★ THE (E) KNOWN-POSITIVE, LIVE. VAS_PUBLISH=assert is a genuine address-plane regression —
#   w298 measured it Xid 31 FAULT_PDE. (E1) and (E2) must both fire on it.
run ekp KAYFABE_VAS_PUBLISH=assert
echo "############ w304 SEQ2 COMPLETE $(date -Is) ############" | tee -a "$S"
