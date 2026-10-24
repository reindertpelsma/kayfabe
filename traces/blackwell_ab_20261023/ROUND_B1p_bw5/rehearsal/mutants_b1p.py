"""Mutant check for the B1p kit changes: each mutant must be killed by >= 2 tests."""

from pathlib import Path
import re
import subprocess
import sys

K = Path(__file__).resolve().parent.parent / "attempt1" / "run_kit"
FIX = "007ba1eb03db9e21d351e465e95b46cbc3134a95"
BASE = "cb80cbd29b7a05f904acd822f172485fd49938a9"
M = [
    ("fix_pin.json", "pin the base", f'"source": "{FIX}"', f'"source": "{BASE}"'),
    ("fix_build.py", "any head", 'if head != pin["source"]:', "if False:"),
    ("fix_build.py", "any parent", 'if _git(repo, "rev-parse", f"HEAD^{n}") != parent:', "if 0:"),
    (
        "fix_build.py",
        "any base",
        'if _git(repo, "rev-parse", f"HEAD^{n}~2") != pin["base"]:',
        "if 0:",
    ),
    ("fix_build.py", "one parent ok", "if len(revs) != 4 or", "if len(revs) < 3 or"),
    ("fix_build.py", "no marker", 'if marker not in _git(repo, "show", "HEAD:" + path):', "if 0:"),
    ("fix_build.py", "dirty ok", '"--untracked-files=no"):', '"--untracked-files=no") and False:'),
    ("fix_build.py", "failed tests ok", 'if "test result: FAILED" in text:', "if False:"),
    ("fix_build.py", "zero tests ok", "if not counts or passes == 0:", "if False:"),
    ("fix_build.py", "no line check", 'if pin["log_line"].encode() not in body:', "if False:"),
    ("fix_build.py", "bin_dir any", 'if pin.get("bin_dir") != pin["source"][:8]:', "if False:"),
    ("fix_build.py", "no binary ok", "if not binary.is_file():", "if False:"),
    (
        "controller.sh",
        "meta sha unchecked",
        "assert hashlib.sha256(Path(j['binary']).read_bytes()).hexdigest()==j['binary_sha256']\n"
        "print(j['binary']);print(j['source'])\nFIX_META",
        "print(j['binary']);print(j['source'])\nFIX_META",
    ),
    (
        "controller.sh",
        "meta any source",
        "assert j['source']==pin['source'] and j['tests_passed'] is True",
        "assert j['tests_passed'] is True",
    ),
    (
        "controller.sh",
        "meta line unchecked",
        "assert j['route_log_line_in_binary'] is True\n",
        "",
    ),
    ("controller.sh", "plan of five", '[ "${#plan[@]}" = 3 ]', '[ "${#plan[@]}" = 5 ]'),
    ("b1n_arms.py", "main arm relay off", '"env": {},', '"env": {"KF3_WIN_USER_RING_RELAY": "0"},'),
    (
        "b1n_arms.py",
        "shadow admitted",
        '"KF3_WIN_USER_RING_RELAY": lambda run, value: value == "0",',
        '"KF3_WIN_USER_RING_RELAY": lambda run, value: value == "0",\n'
        '    "KF3_WIN_USER_RING_SHADOW": lambda run, value: value == "1",',
    ),
    (
        "build_candidates.sh",
        "builds b1",
        "bash /root/sweep2/build_fix.sh",
        "bash /root/sweep2/build_b1.sh",
    ),
    (
        "final_remote_audit.py",
        "audits b1",
        "('observed_fix','/workspace/bw-fix')",
        "('observed_fix','/workspace/bw-observed-b1')",
    ),
]


def killers():
    r = subprocess.run(
        [sys.executable, "-m", "unittest", "test_b1p_kit"],
        cwd=Path(__file__).parent,
        capture_output=True,
        text=True,
    )
    m = re.search(r"FAILED \((?:failures=(\d+))?(?:, )?(?:errors=(\d+))?", r.stderr)
    return int(m[1] or 0) + int(m[2] or 0) if m else 0


for name, label, a, b in M:
    f = K / name
    orig = f.read_text()
    if name == "b1n_arms.py" and label == "main arm relay off":
        assert orig.count(a) == 3, (name, label)
        mutated = orig.replace(a, b, 1)
    else:
        assert orig.count(a) == 1, (name, label)
        mutated = orig.replace(a, b)
    try:
        f.write_text(mutated)
        print(f"{name:22s} {label:22s} killers={killers()}", flush=True)
    finally:
        f.write_text(orig)
