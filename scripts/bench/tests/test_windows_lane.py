"""windows_lane.sh: the stage order that makes it the short path, and what it must not do."""

import pathlib
import re
import subprocess
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "box" / "windows_lane.sh"
TEXT = SCRIPT.read_text()


def calls():
    """The lane's top-level steps, in order (`stage NAME fn` lines and the media-fetch block)."""
    out = []
    for line in TEXT.splitlines():
        m = re.match(r"^stage (\w+) \w+$", line)
        if m:
            out.append(m.group(1))
        elif line.startswith("(cd /root/kayfabe && bash scripts/bench/windows/win_vm.sh fetch"):
            out.append("FETCH_BG")
        elif line.startswith("wait "):
            out.append("FETCH_WAIT")
    return out


class WindowsLane(unittest.TestCase):
    def test_syntax(self):
        r = subprocess.run(["bash", "-n", str(SCRIPT)], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)

    def test_order(self):
        self.assertEqual(
            calls(),
            ["box", "FETCH_BG", "guest_run", "driver", "tree", "kf3", "prereqs", "FETCH_WAIT",
             "install_windows"],
        )

    def test_no_linux_suites_and_no_image_restore(self):
        for word in ("fast_suite", "bare_metal_suite", "v3_gates", "build_fast_guest",
                     "qemu-img convert", "gh release download", "blob-url"):
            self.assertNotIn(word, TEXT.split("set -u", 1)[1], word)

    def test_install_only_uses_the_unsafe_cache_and_caps_cores(self):
        body = TEXT.split("install_windows() {", 1)[1].split("\n}\n", 1)[0]
        self.assertIn("WINVM_INSTALL_CACHE=${WINLANE_INSTALL_CACHE-unsafe}", body)
        self.assertIn('[ "$smp" -le 16 ] || smp=16', body)

    def test_a_failed_stage_stops_the_lane_with_an_exit_line(self):
        self.assertIn('[ "$rc" = 0 ] || { rc_all=$rc; finish; }', TEXT)
        self.assertIn("WINLANE_EXIT rc=", TEXT)

    def test_install_cache_can_be_turned_off_for_one_lease(self):
        body = TEXT.split("install_windows() {", 1)[1].split("\n}\n", 1)[0]
        self.assertIn("WINVM_INSTALL_CACHE=${WINLANE_INSTALL_CACHE-unsafe}", body)

    def test_the_lane_never_carries_per_vm_state_between_boxes(self):
        # the install is made on this box, for this VM; nothing is fetched back, uploaded or restored
        body = TEXT.split("set -u", 1)[1]
        for word in ("secrets", "tpm/", "OVMF_VARS", "known_hosts", "win_password", "base.qcow2",
                     "identity.env", "upload", "rsync", "scp ", "gh release"):
            self.assertNotIn(word, body, word)

    def test_stage_runner_stops_at_the_first_failure_and_ends_with_the_exit_line(self):
        # run exactly the lane's stage/finish text (no root paths, no network) against fake steps
        seg = TEXT[TEXT.index("stage() {") : TEXT.index("box() {")]
        script = "T0=$(date +%s); TIMING=(); rc_all=0\n" + seg + (
            "\nok() { return 0; }\nbad() { return 7; }\nnever() { echo SHOULD_NOT_RUN; }\n"
            "stage one ok\nstage two bad\nstage three never\nstage four ok\n"
        )
        r = subprocess.run(["bash", "-c", script], capture_output=True, text=True)
        self.assertEqual(r.returncode, 7, r.stderr)
        self.assertNotIn("SHOULD_NOT_RUN", r.stdout)
        self.assertNotIn("STAGE three", r.stdout)
        self.assertRegex(r.stdout, r"STAGE one EXIT rc=0 ")
        self.assertRegex(r.stdout, r"STAGE two EXIT rc=7 ")
        self.assertRegex(r.stdout, r"WINLANE_TIMING total=\d+s one=\d+s two=\d+s\n")
        self.assertTrue(r.stdout.rstrip().splitlines()[-1].startswith("WINLANE_EXIT rc=7 "))

    def test_lines_are_at_most_100_columns(self):
        for n, line in enumerate(TEXT.splitlines(), 1):
            self.assertLessEqual(len(line), 100, f"line {n}")


if __name__ == "__main__":
    unittest.main()
