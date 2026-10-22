#!/usr/bin/env python3
"""Run the six declared Rust fault-arm mutants and restore source after each.

The caller owns the build reservation and CARGO_TARGET_DIR. On the build farm,
run from its pushed checkout with --jobs 8. On the dev box, hold the documented
cargo flock and invoke this script under nice 19 with --jobs 2. No GPU is used.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import subprocess

MUTANTS = (
    ("one-shot-removed",
     "self\n                .claimed\n                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)\n                .is_ok()", "true"),
    ("trigger-ignored", "candidate.put == Some(TRIGGER_PUT)", "true"),
    ("native-user-check-removed", "if !port.user() {", "if false {"),
    ("bad-opcode-replaced", "pub const BAD_WORD: u32 = 6 << 29;", "pub const BAD_WORD: u32 = 0;"),
    ("push-error-ignored", "port.push(&[BAD_WORD])?;", "let _ = port.push(&[BAD_WORD]);"),
    ("doorbell-before-store", "port.push(&[BAD_WORD])?;\n    port.ring()",
     "port.ring()?;\n    port.push(&[BAD_WORD])"),
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True, help="new lane-owned evidence directory")
    parser.add_argument("--jobs", type=int, choices=(2, 8), default=2)
    parser.add_argument("--cargo-prefix", default="", help="optional command prefix, parsed without a shell")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    source = root / "crates/kf-chan/src/rc_negative.rs"
    if subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=root):
        raise RuntimeError("pushed, clean source required")
    if not os.environ.get("CARGO_TARGET_DIR"):
        raise RuntimeError("explicit lane-owned CARGO_TARGET_DIR required")
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    args.out.mkdir(parents=True, exist_ok=False)
    original = source.read_bytes()
    report = {"head": head, "hardware_runs": 0, "source_sha256": hashlib.sha256(original).hexdigest(),
              "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(), "mutants": []}
    command = shlex.split(args.cargo_prefix) + [
        "cargo", "test", "-p", "kf-chan", "--features", "rc-negative-control",
        "--test", "rc_negative", "--release", "-j" + str(args.jobs)]
    report["command"] = command

    def run(name):
        st = os.statvfs(root)
        if st.f_bavail * st.f_frsize < 4 * 1024**3:
            raise RuntimeError("disk guard: less than 4 GiB free")
        result = subprocess.run(command, cwd=root, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, text=True)
        (args.out / (name + ".log")).write_text(result.stdout)
        return result

    def green(name):
        result = run(name)
        match = re.search(r"test result: ok\. (\d+) passed; 0 failed;", result.stdout)
        if result.returncode or match is None or int(match[1]) == 0:
            raise RuntimeError(name + " control failed; inspect its log")
        report[name] = {"passed": int(match[1]), "failed": 0}

    try:
        green("before")
        for name, old, new in MUTANTS:
            text = original.decode()
            if text.count(old) != 1:
                raise RuntimeError("mutation site differs: " + name)
            changed = text.replace(old, new).encode()
            try:
                source.write_bytes(changed)
                result = run(name)
            finally:
                source.write_bytes(original)
            killers = re.findall(r"^test (\w+) \.{3} FAILED$", result.stdout, re.M)
            row = {"mutant": name, "mutant_sha256": hashlib.sha256(changed).hexdigest(),
                   "exit_code": result.returncode, "killers": killers, "killed": len(killers)}
            report["mutants"].append(row)
            print(json.dumps({"head": head, **row}), flush=True)
            if result.returncode == 0 or len(killers) < 2:
                raise RuntimeError("mutant requires at least two failing tests: " + name)
        green("after")
        report["status"] = "PASS"
    finally:
        source.write_bytes(original)
        report["source_restored"] = source.read_bytes() == original
        (args.out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"head": head, "status": "PASS", "mutants": len(report["mutants"]),
                      "green_tests": report["after"]["passed"]}), flush=True)


if __name__ == "__main__":
    main()
