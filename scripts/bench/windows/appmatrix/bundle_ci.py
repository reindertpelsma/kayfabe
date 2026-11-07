#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Credential-free planning, disk admission and two-build CI verification."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import package_bundle as B
import bundle_compliance as C


def read_pin(text):
    result = {}
    for line in text.splitlines():
        match = re.fullmatch(r"winapps-open\.iso sha256=([0-9a-f]{64})", line)
        if match:
            if "sha256" in result: raise ValueError("duplicate image pin")
            result["sha256"] = match[1]
        match = re.fullmatch(r"# inputs=([0-9a-f]{64})", line)
        if match:
            if "inputs" in result: raise ValueError("duplicate input pin")
            result["inputs"] = match[1]
    if result and "sha256" not in result:
        raise ValueError("input pin without an image pin")
    return result


def plan(inputs, pin, verify=False):
    if not re.fullmatch(r"[0-9a-f]{64}", inputs): raise ValueError("invalid input digest")
    published = read_pin(pin)
    if verify and "sha256" not in published: raise ValueError("verify requires an existing image pin")
    return dict(inputs=inputs, needs_build=verify or published.get("inputs") != inputs,
                expected_sha256=published.get("sha256", ""), pinned_inputs=published.get("inputs", ""))


def disk_budget(man, profile):
    # Cache + two images + compiler/unpacked runtime/caches/metadata and margin.
    packages = B.select_packages(man, profile)
    payload = sum(f["size"] for p in packages for f in p["files"])
    compliance = sum(pin["size"] for _, pin, _ in C.inputs(packages))
    # Compliance input cache + two ISO copies + one ephemeral source upload file;
    # Reserve the legal-text cap for each package in BOTH images, in addition
    # to compiler/build scratch. The full pinned payload is not staged locally.
    return 3 * payload + 4 * compliance + 2 * len(packages) * C.MAX_TOTAL + 2 * 1024**3


def check_disk(root, required):
    free = shutil.disk_usage(root).free
    print(f"BUNDLE_DISK free={free} required={required}", flush=True)
    if free < required:
        raise ValueError("insufficient disk after cleanup; use a larger runner")


def build_twice(root, expected_inputs, pin, verify=False):
    B.guard(allow_ci=True)
    man, profile = B.load(HERE / "manifest.json"), B.load(HERE / "gate_profiles.json")
    if B.input_hash(man, profile) != expected_inputs:
        raise ValueError("checkout inputs changed after planning")
    published = read_pin(pin)
    if verify and "sha256" not in published: raise ValueError("verify requires an existing image pin")
    root = Path(root)
    if root.exists(): raise ValueError("CI workspace must be fresh")
    root.mkdir(parents=True)
    check_disk(root, disk_budget(man, profile))
    args = ["--ci-build", "--dl", str(root / "downloads")]
    if B.main(["fetch", *args]): raise ValueError("pinned fetch failed")
    receipts = []
    old_mask = os.umask(0o022)
    old_env = {k: os.environ.get(k) for k in ("TZ", "LC_ALL")}
    try:
        for label, mask, tz, loc in (("first", 0o022, "Pacific/Honolulu", "C"), ("second", 0o077, "Asia/Tokyo", "C.UTF-8")):
            os.umask(mask); os.environ.update(TZ=tz, LC_ALL=loc)
            output = root / label
            if B.main(["build", *args, "--out", str(output)]): raise ValueError("bundle build failed")
            receipt = B.load(output / "winapps-open.manifest.json")
            if receipt.get("inputs") != expected_inputs: raise ValueError("build input hash differs from plan")
            receipts.append(receipt)
    finally:
        os.umask(old_mask)
        for k, v in old_env.items():
            if v is None: os.environ.pop(k, None)
            else: os.environ[k] = v
    if receipts[0]["image"]["sha256"] != receipts[1]["image"]["sha256"]:
        raise ValueError("two-build ISO hashes differ")
    B.verify_image(root / "second/winapps-open.iso", receipts[0])
    actual = receipts[0]["image"]["sha256"]
    matches = actual == published.get("sha256")
    print(f"BUNDLE_REPRODUCIBLE sha256={actual} inputs={expected_inputs} pin_match={str(matches).lower()}")
    if verify and not matches:
        raise ValueError("verification rebuild differs from repository pin")
    if not matches: print("BUNDLE_NEW_HASH " + actual)
    return actual


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("command", choices=("plan", "build", "clean"))
    ap.add_argument("--root", type=Path)
    ap.add_argument("--inputs")
    ap.add_argument("--pin", type=Path, default=HERE / "bundle.pin")
    ap.add_argument("--verify", action="store_true")
    a = ap.parse_args(argv)
    try:
        B.guard(allow_ci=True)
        if a.command == "plan":
            inputs = B.input_hash(B.load(HERE / "manifest.json"), B.load(HERE / "gate_profiles.json"))
            data = plan(inputs, a.pin.read_text(encoding="utf-8"), a.verify)
            print(json.dumps(data, sort_keys=True))
            if os.environ.get("GITHUB_OUTPUT"):
                with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as f:
                    for k, v in data.items(): f.write(f"{k}={str(v).lower() if isinstance(v, bool) else v}\n")
        elif a.command == "build":
            if not a.root or not a.inputs: raise ValueError("build requires --root and --inputs")
            build_twice(a.root, a.inputs, a.pin.read_text(encoding="utf-8"), a.verify)
        else:
            # Only the workflow-owned ephemeral directory may be removed.
            temp = Path(os.environ["RUNNER_TEMP"]).resolve()
            if not a.root or a.root.resolve().parent != temp or not a.root.name.startswith("kf-bundle-"):
                raise ValueError("cleanup root is outside the workflow workspace")
            if a.root.exists(): shutil.rmtree(a.root)
        return 0
    except (OSError, ValueError, KeyError) as e:
        print("BUNDLE_CI_FAIL " + str(e), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
