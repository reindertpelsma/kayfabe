"""Apply the 02:30 grant variant to a wired B1g attempt workspace (substitution only).

Usage: variant_b1g.py <D> <K> <default|7vis> <pin machine id|none>
default: the standard profile (offer floor 16, guest SMP 8, original build parallelism,
         150 min ceiling, preflight VERDICT=SOUND required) - no CPU deviation.
7vis:    the B1f "7-visible-core deviation" profile as built.
Both: grant selection override (grant_b1g.select) and an explicit no-match exit after it.
"""

import json
from pathlib import Path
import shutil
import sys

D, K, variant, pin = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3], sys.argv[4]
assert variant in ("default", "7vis")
machine = None if pin == "none" else int(pin)
COMMON = Path("/workspace/bw3-round-b1g-common")
shutil.copy2(COMMON / "grant_b1g.py", D / "grant_b1g.py")
(D / "grant_pin.json").write_text(json.dumps({"machine": machine, "variant": variant}) + "\n")


def edit(path, reps):
    s = path.read_text()
    for a, b in reps:
        assert s.count(a) >= 1, (path.name, a)
        s = s.replace(a, b)
    path.write_text(s)


ops = [
    ("import lease_profile as LP", "import lease_profile as LP\nimport grant_b1g as GRANT"),
    (
        "    eligible = LP.prefer(eligible)\n",
        "    eligible = GRANT.select(eligible)  # controller 02:30 selection override\n"
        "    if not eligible:\n"
        '        print("OBSERVATION NO_QUALIFYING_OFFER_ONE_MARKET_CHECK", flush=True)\n'
        "        sys.exit(75)\n",
    ),
]
if variant == "default":
    ops += [
        ("floor = LP.offer_floor(LP.PROFILE)", "floor = LP.offer_floor(None)  # default profile"),
        ("LP.offer_ok(x, LP.PROFILE)", "LP.offer_ok(x, None)"),
        ("max_seconds=LP.MAX_SECONDS", "max_seconds=None"),
        ("LP.timer_ceiling_seconds(LP.MAX_SECONDS)", "LP.timer_ceiling_seconds(None)"),
        (
            '        **LP.deviation_fields(None, offer["cpu_cores_effective"]),\n',
            '        "lease_profile": "default",\n',
        ),
    ]
edit(D / "ops.py", ops)
if variant == "default":
    edit(
        D / "launch_ready.py",
        [
            (
                'assert row["profile"] == "7-visible-core deviation" and row["cpu_profile_floor"] == 7',
                'assert "profile" not in row and row["lease_profile"] == "default"',
            ),
            (
                'assert row["auto_destroy_minutes"] <= 180 and row["budget_cutoff_minutes"] <= 180',
                'assert row["auto_destroy_minutes"] <= 150 and row["budget_cutoff_minutes"] <= 150',
            ),
        ],
    )
    edit(K / "install.sh", [("WINVM_SMP=6  # B1f profile (default 8)", "WINVM_SMP=8")])
    edit(K / "common.sh", [("WINVM_SMP=6", "WINVM_SMP=8")])
    edit(K / "controller.sh", [
        ('CARGO_BUILD_JOBS=7  # B1f profile "7-visible-core deviation" (default 8)',
         "CARGO_BUILD_JOBS=8")])
    edit(K / "bind.sh", [("CARGO_BUILD_JOBS=7", "CARGO_BUILD_JOBS=16")])
    edit(K / "build_b1.sh", [("CARGO_BUILD_JOBS=7", "CARGO_BUILD_JOBS=8"),
                             ("cargo test -j7", "cargo test -j8")])
print("VARIANT", variant, "pin", machine, D)
