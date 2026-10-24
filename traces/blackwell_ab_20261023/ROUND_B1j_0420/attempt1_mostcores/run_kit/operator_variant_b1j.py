"""Apply the B1j budget override and most-cores selection to a wired default workspace."""

from pathlib import Path
import shutil
import sys

D = Path(sys.argv[1])
COMMON = Path("/workspace/bw3-round-b1g-common")
for name in ("budget_override.py", "grant_b1j.py"):
    shutil.copy2(COMMON / name, D / name)
p = D / "ops.py"
s = p.read_text()
reps = [
    (
        "import grant_b1g as GRANT",
        "import grant_b1g as GRANT\nimport budget_override as BO\nimport grant_b1j as GJ",
    ),
    (
        '        and float(x.get("dph_total", 99)) <= 2.50\n',
        "        and BO.price_ok(x)  # B1j: <= 1.00 USD/h\n",
    ),
    (
        'LP.plan(\n                float(candidate["dph_total"]), credit, now(), stop_usd=2.0, cap_usd=2.5, max_seconds=None\n            )',
        'BO.plan(float(candidate["dph_total"]), credit, now())',
    ),
    (
        '"plan": LP.plan(rate, credit, now(), stop_usd=2.0, cap_usd=2.5, max_seconds=None),',
        '"plan": BO.plan(rate, credit, now()),',
    ),
    (
        "    plan = LP.plan(rate, credit, now(), stop_usd=2.0, cap_usd=2.5, max_seconds=None)\n",
        "    plan = BO.plan(rate, credit, now())\n",
    ),
    (
        '        "budget_stop": 2.0,\n        "phase_cap": 2.5,\n',
        '        "budget_stop": BO.STOP_USD,\n        "phase_cap": BO.CAP_USD,\n        **BO.fields(),\n',
    ),
    ("                        < 2.0\n", "                        < BO.STOP_USD\n"),
    (
        "    eligible = GRANT.select(eligible)  # controller 02:30 selection override\n",
        "    eligible = GJ.select(eligible)  # controller 04:20: most cores, default profile\n",
    ),
]
for a, b in reps:
    assert s.count(a) == 1, a
    s = s.replace(a, b)
p.write_text(s)
print("VARIANT_B1J", D)
