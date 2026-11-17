"""Final exact kernel census must equal source-approved negative-control records."""

import json
from pathlib import Path
import sys

sys.path.insert(0, "/root/kayfabe/scripts/bench")
import native_xid_guard as guard

root = Path("/root/sw4/evidence")
state = json.loads((root / "native-receipt.json").read_text())
accepted = state.get("accepted_xid_lines", state["approved_native_xid_lines"])
actual = guard.xid_lines(guard.kernel_snapshot())
if list(actual) != list(accepted):
    raise ValueError("final unmatched Xid census")
(root / "final_xids.json").write_text(
    json.dumps(
        {
            "final_equal_accepted": True,
            "accepted": len(accepted),
            "completed_arms": len(state.get("completed_arms", [])),
        },
        indent=2,
    )
    + "\n"
)
