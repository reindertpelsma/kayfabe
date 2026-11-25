#!/usr/bin/env python3
"""The release archive holds no control-switch name; the bench archive holds them. STATUS: LIVE, 2026-11-25.

Policy: the release binary has no `KF3_NEGCTL_*` control switch (other environment switches remain; see the S2-02 note
in docs/audits/2026-11-23-v3-stage2.md). The `KF3_NEGCTL_*` environment reads are compiled in
only by the cargo feature `negctl` (crates/kf-qemu, forwarded to crates/kf-host); the release kit never sets it. This
builds the Rust staticlib of kf-qemu twice, with default features and with `--features negctl`, and scans each archive
for the name prefix: a default archive that holds it is a failure, and so is a bench archive that does not (the scan
would then prove nothing).

    python3 -B -I scripts/ci/negctl_artifact.py
"""

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# Assembled at run time: this file is not what the scan finds.
NEEDLE = ("KF3_NEG" + "CTL_").encode()


def holds_name(path):
    """True when the file holds the control-switch name prefix."""
    return NEEDLE in Path(path).read_bytes()


def build_archive(features):
    """Build kf-qemu (dev profile: the strings are the same) and return its staticlib path."""
    cmd = ["cargo", "build", "--locked", "-p", "kf-qemu", "--message-format=json-render-diagnostics"]
    if features:
        cmd += ["--features", features]
    proc = subprocess.run(cmd, cwd=ROOT, stdout=subprocess.PIPE, text=True, check=False)
    if proc.returncode != 0:
        raise SystemExit("cargo build %s failed (%d)" % (" ".join(cmd[2:]), proc.returncode))
    found = []
    for line in proc.stdout.splitlines():
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        if msg.get("reason") == "compiler-artifact" and msg.get("target", {}).get("name") == "kf_qemu":
            found += [f for f in msg.get("filenames", []) if f.endswith(".a")]
    if not found:
        raise SystemExit("cargo reported no staticlib for kf-qemu")
    return found[-1]


def main():
    bad = []
    for features, want in ((None, False), ("negctl", True)):
        archive = build_archive(features)
        got = holds_name(archive)
        label = "control (--features negctl)" if features else "default (the release configuration)"
        print("%s: archive %s the control-switch name" % (label, "holds" if got else "does not hold"))
        if got != want:
            bad.append(label)
    if bad:
        print("FAIL: " + "; ".join(bad), file=sys.stderr)
        return 1
    print("PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
