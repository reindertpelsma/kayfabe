"""BW4 round B1n arm plan (owner 2026-10-24), pinned in priority order; box side.

Every arm is its own boot of the sealed 580.88 parent with the B1 binary (cb80cbd29) and the
unchanged 19 flags. Each arm changes exactly one thing, or nothing (the trace arm only adds a
measurement-only flag):
  on_urt       KF3_USER_RING_TRACE=<arm dir>/urt (existing flag, measures only)   MEASUREMENT
  on_relay_off KF3_WIN_USER_RING_RELAY=0 (existing diagnostic falsifier)          DIAGNOSTIC
  on_tdr60     guest registry TdrDdiDelay=60, set in a standard-VGA prep boot     DIAGNOSTIC
  lx_reload    Linux guest driver unload/reload (second guest RM life)            DIAGNOSTIC
  on_diag_pnp  pnputil disable/enable once after Code 0 (second enable)           DIAGNOSTIC
Usage: b1n_arms.py list | prepare RUN | probed-die LOG EXPECTED OUT
"""

import json
from pathlib import Path
import re
import shlex
import sys

PROV = Path("/root/prov")
RUN = re.compile(r"[a-z0-9_]{1,40}")
DIE = re.compile(r"^PROBE_DIE arch=0x[0-9a-f]+ impl=0x[0-9a-f]+ die=(GB20[2-7])$")
ARMS = (
    {
        "run": "on_urt",
        "kind": "windows",
        "label": "MEASUREMENT",
        "env": {"KF3_USER_RING_TRACE": "/root/prov/arms/on_urt/urt"},
        "prep": None,
        "enable": "enable580_once.ps1",
        "enable_count": 1,
    },
    {
        "run": "on_relay_off",
        "kind": "windows",
        "label": "DIAGNOSTIC",
        "env": {"KF3_WIN_USER_RING_RELAY": "0"},
        "prep": None,
        "enable": "enable580_once.ps1",
        "enable_count": 1,
    },
    {
        "run": "on_tdr60",
        "kind": "windows",
        "label": "DIAGNOSTIC",
        "env": {},
        "prep": "prep_tdr60.ps1",
        "enable": "enable580_once.ps1",
        "enable_count": 1,
    },
    {
        "run": "lx_reload",
        "kind": "linux",
        "label": "DIAGNOSTIC",
        "env": {},
        "prep": None,
        "enable": None,
        "enable_count": 0,
    },
    {
        "run": "on_diag_pnp",
        "kind": "windows",
        "label": "DIAGNOSTIC",
        "env": {},
        "prep": None,
        "enable": "enable580_cycle.ps1",
        "enable_count": 2,
    },
)
ENV_VALUES = {
    "KF3_USER_RING_TRACE": lambda run, value: value == f"/root/prov/arms/{run}/urt",
    "KF3_WIN_USER_RING_RELAY": lambda run, value: value == "0",
}


def validate(arms=ARMS):
    seen = set()
    for arm in arms:
        run = arm["run"]
        if not RUN.fullmatch(run) or run in seen:
            raise ValueError("arm name invalid or repeated: " + run)
        seen.add(run)
        if arm["kind"] == "windows":
            if not run.startswith("on_"):
                raise ValueError("a Windows arm must be an on_ arm")
            if arm["enable"] not in ("enable580_once.ps1", "enable580_cycle.ps1"):
                raise ValueError("unknown enable script")
            cycle = arm["enable"] == "enable580_cycle.ps1"
            if arm["enable_count"] != (2 if cycle else 1):
                raise ValueError("enable count does not match the enable script")
            if cycle and arm["label"] != "DIAGNOSTIC":
                raise ValueError("a second enable is DIAGNOSTIC only")
        elif arm["kind"] == "linux":
            if run.startswith("on_") or arm["enable"] or arm["enable_count"] != 0:
                raise ValueError("a Linux arm has no Windows enable and no observation permit")
            if arm["env"] or arm["prep"]:
                raise ValueError("a Linux arm carries no flag and no guest prep")
        else:
            raise ValueError("unknown arm kind")
        if arm["label"] not in ("MEASUREMENT", "DIAGNOSTIC"):
            raise ValueError("arm label missing")
        if arm["prep"] not in (None, "prep_tdr60.ps1"):
            raise ValueError("unknown prep script")
        if arm["prep"] and arm["label"] != "DIAGNOSTIC":
            raise ValueError("a guest behaviour change is DIAGNOSTIC")
        for key, value in arm["env"].items():
            check = ENV_VALUES.get(key)
            if check is None or not check(run, value):
                raise ValueError("flag not allowed for this arm: " + key)
        if arm["env"] and arm["label"] == "MEASUREMENT":
            if set(arm["env"]) != {"KF3_USER_RING_TRACE"}:
                raise ValueError("only the trace flag is measurement-only")
        changes = len(arm["env"]) + bool(arm["prep"]) + (arm["enable_count"] == 2)
        if changes > 1:
            raise ValueError("an arm changes at most one thing")
    return True


def windows_arms(arms=ARMS):
    return [arm["run"] for arm in arms if arm["kind"] == "windows"]


def diagnostic_enable_arms(arms=ARMS):
    return {arm["run"]: 2 for arm in arms if arm["enable_count"] == 2}


def find(run, arms=ARMS):
    validate(arms)
    for arm in arms:
        if arm["run"] == run:
            return arm
    raise ValueError("arm is not in the pinned B1n plan: " + run)


def env_text(arm):
    lines = [f"export {key}={shlex.quote(value)}" for key, value in sorted(arm["env"].items())]
    return "".join(line + "\n" for line in lines)


def permit_fields(run, probed, arms=ARMS):
    arm = find(run, arms)
    if arm["kind"] != "windows":
        raise ValueError("only a Windows arm carries an observation permit")
    die = json.loads(Path(probed).read_text())["die"]
    return {"probed_die": die, "enable_count": arm["enable_count"], "label": arm["label"]}


def probed_die(log_text, expected):
    found = [m[1] for m in (DIE.match(line.strip()) for line in log_text.splitlines()) if m]
    if len(found) != 1:
        raise ValueError("exactly one PROBE_DIE line is required")
    if found[0] != expected:
        raise ValueError("probed die differs from the lease candidate die")
    return found[0]


def prepare(run, prov=PROV, arms=ARMS):
    arm = find(run, arms)
    prov = Path(prov)
    (prov / f"arm_env_{run}.sh").write_text(env_text(arm))
    (prov / f"arm_cfg_{run}.json").write_text(json.dumps(arm, indent=2) + "\n")
    return f"{arm['enable'] or '-'} {arm['enable_count']} {arm['prep'] or '-'} {arm['label']}"


def main(argv):
    if argv[:1] == ["list"]:
        validate()
        print("\n".join(f"{arm['run']} {arm['kind']}" for arm in ARMS))
    elif argv[:1] == ["prepare"] and len(argv) == 2:
        print(prepare(argv[1]))
    elif argv[:1] == ["probed-die"] and len(argv) == 4:
        die = probed_die(Path(argv[1]).read_text(), argv[2])
        Path(argv[3]).write_text(json.dumps({"die": die, "source": argv[1]}) + "\n")
        print("PROBED_DIE " + die)
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
