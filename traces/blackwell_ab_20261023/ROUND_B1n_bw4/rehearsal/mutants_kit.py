"""Mutant check for the B1n kit modules: each mutant must be killed by >= 2 tests."""

from pathlib import Path
import re
import subprocess
import sys

K = Path(__file__).resolve().parent.parent / "attempt1" / "run_kit"
M = [
    ("b1n_arms.py", "trace dir any", 'value == f"/root/prov/arms/{run}/urt"', "True"),
    (
        "b1n_arms.py",
        "relay any value",
        'lambda run, value: value == "0"',
        "lambda run, value: True",
    ),
    ("b1n_arms.py", "two changes ok", "if changes > 1:", "if changes > 2:"),
    ("b1n_arms.py", "cycle any label", 'if cycle and arm["label"] != "DIAGNOSTIC":', "if False:"),
    (
        "b1n_arms.py",
        "linux on_ ok",
        'if run.startswith("on_") or arm["enable"]',
        "if arm['enable']",
    ),
    ("b1n_arms.py", "die any", "if found[0] != expected:", "if False:"),
    ("b1n_arms.py", "die many", "if len(found) != 1:", "if not found:"),
    ("b1n_authority.py", "authority any die", "if die not in table:", "if False:"),
    (
        "b1n_authority.py",
        "linux in arms",
        '"arms": b1n_arms.windows_arms(),',
        '"arms": [a["run"] for a in b1n_arms.ARMS],',
    ),
    (
        "capture_pack.py",
        "no mismatch",
        '                mismatches.append(name[len("guest/") :])',
        "                pass",
    ),
    (
        "capture_pack.py",
        "other arm logs",
        'for path in sorted(vm_logs.glob(run + "_*")):\n'
        '        if path.is_file() and re.fullmatch(re.escape(run) + r"_[A-Za-z0-9_.-]+", path.name):',
        'for path in sorted(vm_logs.glob("*")):\n        if path.is_file():',
    ),
    (
        "capture_pull.py",
        "no hash check",
        "ok = target.is_file() and sha256(target) == digest",
        "ok = target.is_file()",
    ),
    ("capture_pull.py", "escape ok", 'or ".." in path.parts', "or False"),
    (
        "capture_pull.py",
        "repo ok",
        'if any(part.endswith("-wt") or part == "traces" for part in base.parts):',
        "if False:",
    ),
    ("capture_pull.py", "no deadline", "if remaining <= 0:", "if False:"),
    (
        "fatal_collect.py",
        "no marker ok",
        "    if not marker.exists():\n        raise",
        "    if False:\n        raise",
    ),
    ("fatal_collect.py", "live qemu ok", "    if qemu_running:\n", "    if False:\n"),
    ("fatal_collect.py", "kf3 arm", '"/root/sweep2/collect_bw.sh"', '"/root/sweep2/arm.sh"'),
    (
        "fatal_collect.py",
        "linux collects",
        "        return arm if type(arm) is str and RUN.fullmatch(arm) and arm in disks else None",
        "        pass",
    ),
    (
        "fatal_collect.py",
        "history change ok",
        "    if list(after[: len(before)]) != list(before):",
        "    if False:",
    ),
    (
        "capture_pack.py",
        "unordered manifest",
        'order = sorted(files.items(), key=lambda item: (item[1]["bytes"], item[0]))',
        "order = sorted(files.items())",
    ),
    (
        "capture_pull.py",
        "stream trusted",
        "        if not (target.is_file() and sha256(target) == digest):",
        "        if not target.is_file():",
    ),
    (
        "capture_pull.py",
        "no fallback",
        'retry = pull_arm(run, "".join(f"{d}  {n}\\n" for d, n in bad), dest_root, fetch, remaining)',
        'retry = pull_arm(run, "", dest_root, fetch, remaining)',
    ),
]


def killers():
    r = subprocess.run(
        [sys.executable, "-m", "unittest", "test_b1n_kit"],
        cwd=Path(__file__).parent,
        capture_output=True,
        text=True,
    )
    m = re.search(r"FAILED \((?:failures=(\d+))?(?:, )?(?:errors=(\d+))?", r.stderr)
    return int(m[1] or 0) + int(m[2] or 0) if m else 0


for name, label, a, b in M:
    f = K / name
    orig = f.read_text()
    assert orig.count(a) == 1, (name, label)
    try:
        f.write_text(orig.replace(a, b))
        print(f"{name:18s} {label:22s} killers={killers()}", flush=True)
    finally:
        f.write_text(orig)
