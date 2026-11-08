#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Public-only planning, disk admission and credential-free two-build verification."""
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
import bundle_repro as R


def read_pin(text, bundle_class="public"):
    if bundle_class != "public": raise ValueError("CI release planning is public-only")
    filename = "winapps-open"
    result = {}
    for line in text.splitlines():
        match = re.fullmatch(filename + r"\.iso sha256=([0-9a-f]{64})", line)
        if match:
            if "sha256" in result: raise ValueError("duplicate image pin")
            result["sha256"] = match[1]
        match = re.fullmatch(r"# inventory=([0-9a-f]{64})", line)
        if match:
            if "inventory" in result: raise ValueError("duplicate inventory pin")
            result["inventory"] = match[1]
        match = re.fullmatch(r"# inputs=([0-9a-f]{64})", line)
        if match:
            if "inputs" in result: raise ValueError("duplicate input pin")
            result["inputs"] = match[1]
    if result and "sha256" not in result:
        raise ValueError("input pin without an image pin")
    return result


def plan(inputs, pin, verify=False, bundle_class="public", receipt=None):
    if not re.fullmatch(r"[0-9a-f]{64}", inputs): raise ValueError("invalid input digest")
    published = read_pin(pin, bundle_class)
    if receipt is not None:
        if receipt.get("bundle_class") != bundle_class or receipt.get("image", {}).get("sha256") != published.get("sha256"):
            raise ValueError("published manifest differs from selected class/pin")
        if not re.fullmatch(r"[0-9a-f]{64}", receipt.get("inputs", "")): raise ValueError("invalid published input hash")
        published["inputs"] = receipt["inputs"]
    if verify and "sha256" not in published: raise ValueError("verify requires an existing image pin")
    return dict(inputs=inputs, needs_build=verify or published.get("inputs") != inputs,
                expected_sha256=published.get("sha256", ""), pinned_inputs=published.get("inputs", ""))


def disk_budget(man, profile, bundle_class="public"):
    # Cache + two images + compiler/unpacked runtime/caches/metadata and margin.
    packages = B.select_packages(man, profile, bundle_class=bundle_class)
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


def build_twice(root, expected_inputs, pin, verify=False, bundle_class="public", reference=None):
    if bundle_class != "public": raise ValueError("CI release builds are public-only")
    basename = "winapps-open"
    B.guard(allow_ci=True)
    man, profile = B.load(HERE / "manifest.json"), B.load(HERE / "gate_profiles.json")
    if B.input_hash(man, profile, bundle_class=bundle_class) != expected_inputs:
        raise ValueError("checkout inputs changed after planning")
    published = read_pin(pin, bundle_class)
    if verify and "sha256" not in published: raise ValueError("verify requires an existing image pin")
    if verify and reference is None and not published.get("inventory"):
        raise ValueError("verification needs a Level 1 inventory pin or the published file manifest")
    if reference is not None:
        if reference['image']['sha256'] != published.get('sha256'): raise ValueError('reference manifest differs from repository ISO pin')
        if published.get('inventory') and R.inventory_hash(reference) != published['inventory']: raise ValueError('reference inventory differs from repository pin')
    root = Path(root)
    if root.exists(): raise ValueError("CI workspace must be fresh")
    root.mkdir(parents=True)
    check_disk(root, disk_budget(man, profile, bundle_class))
    args = ["--ci-build", "--class", bundle_class, "--dl", str(root / "downloads")]
    if B.main(["fetch", *args]): raise ValueError("pinned fetch failed")
    receipts = []
    old_mask = os.umask(0o022)
    old_env = {k: os.environ.get(k) for k in ("TZ", "LC_ALL")}
    try:
        for label, mask, tz, loc in (("first", 0o022, "Pacific/Honolulu", "C"), ("second", 0o077, "Asia/Tokyo", "C.UTF-8")):
            os.umask(mask); os.environ.update(TZ=tz, LC_ALL=loc)
            output = root / label
            if B.main(["build", *args, "--out", str(output)]): raise ValueError("bundle build failed")
            receipt = B.load(output / (basename + ".manifest.json"))
            if receipt.get("inputs") != expected_inputs: raise ValueError("build input hash differs from plan")
            receipts.append(receipt)
    finally:
        os.umask(old_mask)
        for k, v in old_env.items():
            if v is None: os.environ.pop(k, None)
            else: os.environ[k] = v
    # Read each container against ITS OWN receipt. A cross-receipt ISO digest
    # check would turn harmless metadata differences into a Level 1 failure.
    first = R.read_candidate(root / "first" / (basename + ".iso"), receipts[0])
    second = R.read_candidate(root / "second" / (basename + ".iso"), receipts[1])
    comparison = R.compare(first, second)
    R.report(comparison)
    (root / 'reproduction.json').write_bytes(B.canonical(comparison))
    if comparison['level1'] != 'equal': raise ValueError('LEVEL1_FAIL two-build file inventory differs')
    actual = first['image']['sha256']
    if verify:
        if reference is not None:
            comparison = R.compare(reference, first)
            R.report(comparison)
            if comparison['level1'] != 'equal': raise ValueError('LEVEL1_FAIL rebuilt files differ from published manifest')
        elif R.inventory_hash(first) != published.get('inventory'):
            raise ValueError('LEVEL1_FAIL rebuilt inventory digest differs from repository pin')
        if actual != published['sha256'] and reference is None:
            print('WARNING LEVEL2_CONTAINER_DIFF '+json.dumps(dict(expected_sha256=published['sha256'],actual_sha256=actual,metadata='baseline unavailable; supply the published manifest for field diff'),sort_keys=True))
    print(f"BUNDLE_LEVEL1_REPRODUCIBLE sha256={actual} inputs={expected_inputs} inventory={R.inventory_hash(first)}")
    if actual != published.get('sha256'): print('BUNDLE_NEW_HASH '+actual)

    return actual


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("command", choices=("plan", "build", "clean"))
    ap.add_argument("--root", type=Path)
    ap.add_argument("--inputs")
    ap.add_argument("--pin", type=Path, default=None)
    ap.add_argument("--verify", action="store_true")
    ap.add_argument("--class", dest="bundle_class", choices=("public",), default="public")
    ap.add_argument("--receipt", type=Path)
    a = ap.parse_args(argv)
    try:
        if a.pin is None: a.pin = HERE / "bundle.pin"
        B.guard(allow_ci=True)
        if a.command == "plan":
            inputs = B.input_hash(B.load(HERE / "manifest.json"), B.load(HERE / "gate_profiles.json"), bundle_class=a.bundle_class)
            receipt = B.load(a.receipt) if a.receipt else None
            pin_text = a.pin.read_text(encoding="utf-8")
            digest = read_pin(pin_text, a.bundle_class).get("sha256")
            if a.bundle_class == "public" and digest and os.environ.get("BUNDLE_PUBLIC_BASE_URL"):
                import publish_bundle as P
                try:
                    _, body = P.send("GET", P.public_base(os.environ) + "/" + P.keys(digest)[1], {}, b"")
                    receipt = json.loads(body)
                except P.StoreError as error:
                    if error.status != 404: raise
            data = plan(inputs, pin_text, a.verify, a.bundle_class, receipt)
            print(json.dumps(data, sort_keys=True))
            if os.environ.get("GITHUB_OUTPUT"):
                with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as f:
                    for k, v in data.items(): f.write(f"{k}={str(v).lower() if isinstance(v, bool) else v}\n")
        elif a.command == "build":
            if not a.root or not a.inputs: raise ValueError("build requires --root and --inputs")
            pin_text = a.pin.read_text(encoding="utf-8")
            reference = B.load(a.receipt) if a.receipt else None
            if a.verify and reference is None and os.environ.get('BUNDLE_PUBLIC_BASE_URL'):
                import publish_bundle as P
                digest = read_pin(pin_text).get('sha256')
                if digest:
                    _, body = P.send('GET', P.public_base(os.environ)+'/'+P.keys(digest)[1], {}, b'')
                    reference = json.loads(body)
            build_twice(a.root, a.inputs, pin_text, a.verify, a.bundle_class, reference)
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
