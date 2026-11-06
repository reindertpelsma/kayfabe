"""STATUS: LIVE, 2026-11-06. GPU-free test of win_vm.sh `cmd_fetch`: a transfer that fails with
curl rc=56 is resumed (-C -) and retried, a server that cannot resume (rc 33) restarts from zero, and a
medium that never downloads dies after 8 attempts. A fake `curl` stands in for the network."""
import hashlib
import os
import re
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/bench/windows/win_vm.sh"


def fetch_function():
    text = SCRIPT.read_text()
    match = re.search(r"^cmd_fetch\(\) \{\n.*?^\}\n", text, re.S | re.M)
    assert match, "cmd_fetch not found"
    return match.group(0)


FAKE_CURL = """#!/bin/bash
# args: ... -C - -o OUT URL ; behaviour from $FAKE_PLAN (one rc per call, then success)
out=""; resume=0
while [ $# -gt 0 ]; do case "$1" in -o) out=$2; shift;; -C) resume=1;; esac; shift; done
n=$(cat "$FAKE_STATE" 2>/dev/null || echo 0); n=$((n + 1)); echo $n > "$FAKE_STATE"
rc=$(echo "$FAKE_PLAN" | cut -d, -f$n)
echo "call $n resume=$resume out_exists=$([ -e "$out" ] && echo 1 || echo 0) rc=${rc:-0}" >> "$FAKE_LOG"
if [ -n "$rc" ] && [ "$rc" != 0 ]; then printf 'partial' >> "$out"; exit "$rc"; fi
printf 'payload' > "$out"; exit 0
"""


class FetchTest(unittest.TestCase):
    def run_fetch(self, plan):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            bindir = tmp / "bin"
            bindir.mkdir()
            curl = bindir / "curl"
            curl.write_text(FAKE_CURL)
            curl.chmod(curl.stat().st_mode | stat.S_IEXEC)
            sleep = bindir / "sleep"
            sleep.write_text("#!/bin/bash\nexit 0\n")
            sleep.chmod(sleep.stat().st_mode | stat.S_IEXEC)
            digest = hashlib.sha256(b"payload").hexdigest()
            script = (
                "set -euo pipefail\n"
                f'CACHE={tmp}/cache\n'
                f'MEDIA=("m.iso|http://example.invalid/m.iso|{digest}|7")\n'
                'log() { echo "LOG $*"; }\n'
                'die() { echo "DIE $*"; exit 1; }\n'
                + fetch_function()
                + "cmd_fetch\n"
            )
            env = dict(os.environ)
            env.update(PATH=f"{bindir}:{env['PATH']}", FAKE_PLAN=plan, FAKE_STATE=str(tmp / "n"), FAKE_LOG=str(tmp / "calls"))
            res = subprocess.run(["bash", "-c", script], env=env, capture_output=True, text=True)
            calls = (tmp / "calls").read_text() if (tmp / "calls").exists() else ""
            return res, calls

    def test_resumes_after_rc56_and_succeeds(self):
        res, calls = self.run_fetch("56,56,0")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        self.assertIn("MEDIUM_OK m.iso", res.stdout)
        self.assertEqual(calls.count("resume=1"), 3, calls)
        self.assertIn("call 2 resume=1 out_exists=1", calls)

    def test_rc33_restarts_from_zero(self):
        res, calls = self.run_fetch("33,0")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        self.assertIn("call 2 resume=1 out_exists=0", calls)

    def test_gives_up_after_eight_attempts(self):
        res, calls = self.run_fetch("56,56,56,56,56,56,56,56,56")
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("DIE m.iso: download failed after 8 attempts", res.stdout)
        self.assertEqual(calls.count("call "), 8, calls)


if __name__ == "__main__":
    unittest.main()
