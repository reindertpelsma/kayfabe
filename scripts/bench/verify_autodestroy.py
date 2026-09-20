"""Assert every rented vast box has a server-side net that cannot share fate with us.

⊘ Reports only; never destroys. Teardown is scoped to ids you created, and this account is
shared with other sessions — a verifier that destroyed what it did not like would be a worse
failure than the one it detects.
"""

import datetime
import json
import os
import sys


def main() -> int:
    max_h = float(os.environ.get("MAX_H", "168"))
    now = datetime.datetime.now(datetime.timezone.utc).timestamp()
    data = json.load(sys.stdin)
    if not data:
        print("verify_autodestroy: no instances — nothing to cover")
        return 0

    uncovered = 0
    for inst in data:
        iid = inst.get("id")
        status = inst.get("actual_status")
        end = inst.get("end_date")
        if not end:
            # ⊘ The one state that must be loud: nothing will ever stop this box but us.
            print("  X  {} ({}): NO end_date — NO SERVER-SIDE NET".format(iid, status))
            uncovered += 1
            continue
        hours = (end - now) / 3600.0
        when = datetime.datetime.fromtimestamp(end, datetime.timezone.utc).isoformat()
        if hours <= 0:
            print("  X  {} ({}): end_date {} is IN THE PAST, yet it is still listed"
                  .format(iid, status, when))
            uncovered += 1
        elif hours > max_h:
            print("  !  {} ({}): end_date {} is {:.0f}h out (> {:.0f}h)"
                  .format(iid, status, when, hours, max_h))
            uncovered += 1
        else:
            print("  ok {} ({}): end_date {} ({:.1f}h)".format(iid, status, when, hours))

    verdict = "FAIL" if uncovered else "PASS"
    print("VERIFY_AUTODESTROY={} instances={} uncovered={}"
          .format(verdict, len(data), uncovered))
    return 1 if uncovered else 0


if __name__ == "__main__":
    sys.exit(main())
