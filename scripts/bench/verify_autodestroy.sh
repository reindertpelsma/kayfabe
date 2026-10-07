#!/usr/bin/env bash
# ★★★★★ EVERY RENTED BOX HAS A NET THAT DOES NOT SHARE ITS FATE WITH ME.
#
# > Owner, 2026-09-20: *"ensure auto destroy works on all vast boxes."*
#
# ⊘⊘⊘ **THE LOCAL NET IS NOT SUFFICIENT AND THE EVIDENCE IS IN OUR OWN LOGS.**
# `autodestroy_box.sh` is detached with `setsid` and still dies with the workspace:
# `[measured 2026-09-20]` FIVE logs — 51220903, 51221284, 51252979, 51256037, 51402274 —
# carry `=== ARMED ===` and **no `FIRING` line**. Each of those nets was killed before its
# deadline. The script's own header says why: *"Every in-process net SHARES ITS FATE WITH THE
# PROCESS IT PROTECTS."* `setsid` widens the window; it does not change the rule.
#
# ⊘ And the ON-BOX deadman (`.deadman/watch.sh`) cannot work AT ALL under the standing
# constraint *"no secrets on a box"*: there is no `vastai` CLI and no credentials there, so it
# would fail silently forever. `[measured 2026-09-20]` it was not even running.
#
# ⇒ The only net that cannot share fate is the SERVER-SIDE `end_date`. This script asserts it
# exists, is in the future, and is not absurdly far out — for EVERY instance, by enumeration,
# because the question *"is every box covered"* cannot be answered one id at a time.
#
# ⚠ It REPORTS and EXITS NON-ZERO; it never destroys. Teardown is scoped to ids you created
# (memory: `teardown_is_scoped_to_ids_you_created`) and this account is shared with other
# sessions — a verifier that destroyed what it did not like would be a worse failure than the
# one it detects.
# ⊘⊘⊘ **THE SERVER-SIDE DATE CANNOT BE TIGHTENED AFTER CREATION, AND THE API LIES ABOUT IT.**
# `[measured 2026-09-20]` `PUT /api/v0/instances/<id>/` returns `{"success": true}` for
# `{"duration": …}`, `{"end_date": …}` and `{"extend": false, "duration": …}` alike — and
# `end_date` does not move for any of them. ⚠ Do not re-derive this: three bodies, three
# successes, zero change. Same family as everything else this tree records —
# `a_check_that_reports_is_not_a_check_that_gates`, arriving from the vendor.
# ⇒ A box's window is fixed AT RENT TIME. This script therefore FLAGS a long one rather than
# pretending it can fix it, and the exposure is a thing to decide about, not a thing to hide.
set -uo pipefail
MAX_H=${MAX_H:-168}     # louder than a limit: a box with >1 week left is worth a second look
rc=0
raw=$(timeout 120 vastai show instances --raw 2>/dev/null) || {
    echo "verify_autodestroy: ⊘ could not query vast — UNMEASURED, not 'all clear'"; exit 2; }

echo "$raw" | MAX_H="$MAX_H" python3 "$(dirname "$0")/verify_autodestroy.py" || rc=$?
exit $rc
