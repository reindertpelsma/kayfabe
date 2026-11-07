#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free mutation checks: >=2 named assertion failures per changed rule.

Each mutation runs in a scratch copy. A syntax/import error never kills a mutant.
AST locations make mutations independent of formatting; a missing or ambiguous
site is fatal. Only fake QEMU and local fixtures run, with no network or VM.
"""

import argparse
import ast
import concurrent.futures
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
TESTS = {"core": "test_apps_image.py", "layers": "test_apps_image_layers.py"}


def guard(message):
    return ("guard", message)


def finding(message):
    return ("finding", message)


# name, module, AST operation(s), test-file / classes. Test selection saves CPU;
# it does not change the two-killer requirement or allow ERRORs to count.
CASES = [
    (
        "missing-input-hash",
        "common",
        [guard("input needs an id and SHA256 pin")],
        "core",
        ["Pins"],
    ),
    (
        "public-url-check",
        "common",
        [guard("input URL must be HTTPS")],
        "core",
        ["Pins"],
    ),
    (
        "snapshot-check",
        "common",
        [guard("apt snapshot must be immutable")],
        "core",
        ["Pins"],
    ),
    (
        "shell-wrapper-check",
        "common",
        [guard("shell/wrapper commands")],
        "core",
        ["Pins"],
    ),
    (
        "download-digest-check",
        "common",
        [("bool-part", "size/SHA256 mismatch", 1)],
        "core",
        ["Downloads"],
    ),
    (
        "download-size-check",
        "common",
        [("bool-part", "size/SHA256 mismatch", 0)],
        "core",
        ["Downloads"],
    ),
    (
        "cache-reverification",
        "common",
        [("call", "verify_file(target, pin)")],
        "core",
        ["Downloads"],
    ),
    (
        "archive-path-check",
        "common",
        [guard("unsafe relative path")],
        "core",
        ["Archives"],
    ),
    (
        "guest-identity-check",
        "common",
        [
            guard("guest kernel/NVIDIA module differs"),
            guard("NVIDIA module in open image"),
        ],
        "core",
        ["Inventory"],
    ),
    (
        "apt-inventory-completeness",
        "common",
        [guard("missing or wrong-version apt package")],
        "core",
        ["Inventory"],
    ),
    (
        "pip-files-completeness",
        "common",
        [("bool-part", "missing pip installed-file hashes", 0)],
        "core",
        ["Inventory"],
    ),
    (
        "model-inventory-pin",
        "common",
        [guard("file pin differs")],
        "core",
        ["Inventory"],
    ),
    (
        "built-binary-pin",
        "common",
        [guard("build output hash mismatch")],
        "core",
        ["Inventory"],
    ),
    (
        "pip-freeze-check",
        "common",
        [("bool-part", "pip freeze or wheel pins differ", 1)],
        "core",
        ["Inventory", "FakeQemu.test_build_refuses_pip_version_drift"],
    ),
    (
        "open-nvidia-boundary",
        "common",
        [guard("NVIDIA input cannot be part of open artifacts")],
        "layers",
        ["LayerRecipe"],
    ),
    (
        "open-apt-components",
        "common",
        [guard("open image permits only main/universe")],
        "layers",
        ["LayerRecipe"],
    ),
    (
        "open-scan-required",
        "common",
        [guard("NVIDIA-file scan failed")],
        "layers",
        ["LayerInventories"],
    ),
    (
        "official-receipts-required",
        "common",
        [guard("official input receipts differ")],
        "layers",
        ["LayerInventories"],
    ),
    (
        "inventory-comparison",
        "build_image",
        [guard("inventory differs")],
        "core",
        ["Inventory"],
    ),
    (
        "completion-marker",
        "build_image",
        [guard("missing/duplicate/mismatched completion marker")],
        "core",
        [
            "FakeQemu.test_duplicate_completion_marker_refused",
            "FakeQemu.test_marker_with_wrong_inventory_digest_refused",
            "FakeQemu.test_zero_qemu_exit_without_marker_refused",
        ],
    ),
    (
        "fresh-session-check",
        "build_image",
        [("nonce",)],
        "core",
        [
            "FakeQemu.test_stale_completion_refused",
            "FakeQemu.test_verify_rejects_stale_guest_session",
        ],
    ),
    (
        "qemu-exit-status",
        "build_image",
        [guard("QEMU failed with status")],
        "core",
        [
            "FakeQemu.test_nonzero_qemu_exit_refused_despite_complete_report",
            "FakeQemu.test_verify_rejects_nonzero_qemu_exit",
        ],
    ),
    (
        "qemu-img-exit-status",
        "build_image",
        [("check",)],
        "core",
        [
            "FakeQemu.test_qemu_img_check_failure_does_not_publish_manifest",
            "FakeQemu.test_qemu_img_failure_does_not_publish_manifest",
        ],
    ),
    (
        "assembled-export-integrity",
        "build_image",
        [guard("assembled app differs from audited builder export")],
        "core",
        [
            "FakeQemu.test_assembled_app_must_match_the_audited_builder_bytes",
            "FakeQemu.test_assembled_image_cannot_drop_upstream_licence_files",
        ],
    ),
    (
        "guest-pip-hash-check",
        "guest",
        [("argument", "--require-hashes")],
        "core",
        ["GuestInstall"],
    ),
    (
        "guest-pip-index-check",
        "guest",
        [("argument", "--no-index")],
        "core",
        ["GuestInstall"],
    ),
    ("guest-command-status", "guest", [("check",)], "core", ["GuestInstall"]),
    (
        "builder-header-link-boundary",
        "guest",
        [("builder-links",)],
        "layers",
        ["BuilderBoundary"],
    ),
    (
        "opencl-gpu-selection",
        "guest",
        [("opencl-selection",)],
        "layers",
        ["OpenClTarget"],
    ),
    (
        "opencl-nvidia-required",
        "guest",
        [guard("NVIDIA OpenCL ICD was not installed")],
        "layers",
        ["OpenClTarget"],
    ),
    (
        "proprietary-header-scan",
        "scan_image",
        [finding("NVIDIA proprietary licence header")],
        "layers",
        ["Scan", "StrictNoticeScan", "BundledCudaHeaders"],
    ),
    (
        "vendor-payload-path-scan",
        "scan_image",
        [finding("NVIDIA payload path")],
        "layers",
        ["Scan", "StrictNoticeScan"],
    ),
    (
        "vendor-package-scan",
        "scan_image",
        [finding("NVIDIA package:")],
        "layers",
        ["Scan", "StrictNoticeScan"],
    ),
    (
        "unknown-notice-scan",
        "scan_image",
        [finding("NVIDIA notice without documented OSS licence")],
        "layers",
        ["StrictNoticeScan", "KernelOssHeaders"],
    ),
    (
        "official-http-status",
        "official",
        [guard(": HTTP status")],
        "layers",
        ["OfficialFetch"],
    ),
    (
        "official-content-length",
        "official",
        [guard(": length")],
        "layers",
        ["OfficialFetch"],
    ),
    (
        "monthly-failure-status",
        "official",
        [("monthly-failed",)],
        "layers",
        ["OfficialFetch", "AdditionalLayerFailures"],
    ),
    (
        "guest-output-byte-cap",
        "channels",
        [guard("output limit exceeded")],
        "layers",
        ["BoundedTransport"],
    ),
    (
        "shared-runtime-dependency",
        "elf",
        [guard("missing shared runtime dependency")],
        "layers",
        ["ElfLinking"],
    ),
    (
        "apt-mirror-components",
        "apt_mirror",
        [guard("main/universe only")],
        "layers",
        ["FrozenApt"],
    ),
    (
        "apt-pool-boundary",
        "apt_mirror",
        [guard("package outside pool")],
        "layers",
        ["FrozenApt"],
    ),
    (
        "apt-download-budget",
        "apt_mirror",
        [guard("apt fetch budget exceeded")],
        "layers",
        ["FrozenApt"],
    ),
    (
        "export-file-digests",
        "export_check",
        [guard("app export differs from builder inventory")],
        "layers",
        ["ExportTransfer"],
    ),
    (
        "export-cuda-path-boundary",
        "export_check",
        [guard("export outside approved app bundle")],
        "layers",
        ["ExportTransfer"],
    ),
    (
        "export-special-file-boundary",
        "export_check",
        [guard("PAX, special or non-file export refused")],
        "layers",
        ["ExportTransfer"],
    ),
    (
        "export-static-output-boundary",
        "export_check",
        [guard("static NVIDIA-layer output exported")],
        "layers",
        ["ExportTransfer"],
    ),
    (
        "seed-required-files",
        "iso9660",
        [guard("seed requires exactly the three NoCloud files")],
        "core",
        ["Seed"],
    ),
    ("seed-volume-label", "iso9660", [("label",)], "core", ["Seed"]),
    (
        "forward-endpoint-validation",
        "build_image",
        [guard("invalid pinned input forward endpoint")],
        "core",
        ["ForwardedInputs"],
    ),
    (
        "explicit-input-forward",
        "build_image",
        [("forward-rule",)],
        "core",
        ["ForwardedInputs"],
    ),
    (
        "scan-traversal-errors",
        "scan_image",
        [("walk-errors",)],
        "layers",
        ["ScanTraversalErrors"],
    ),
    (
        "scan-resource-bounds",
        "scan_image",
        [
            ("finding-guard", "scan file budget exceeded"),
            ("finding-guard", "scan byte budget exceeded"),
        ],
        "layers",
        ["AdditionalLayerFailures"],
    ),
    (
        "official-disk-writer-digest",
        "input_disk",
        [guard("official disk input digest mismatch")],
        "layers",
        ["OfficialInputDisk"],
    ),
    (
        "official-disk-reader-digest",
        "input_disk",
        [("call", "verify_file(part, pin)")],
        "layers",
        ["OfficialInputDisk"],
    ),
    (
        "official-disk-declared-size",
        "input_disk",
        [guard("official disk declared size differs from pin")],
        "layers",
        ["OfficialInputDisk"],
    ),
    (
        "official-disk-existing-file",
        "input_disk",
        [("created-cleanup",)],
        "layers",
        ["OfficialInputDisk"],
    ),
    (
        "official-guest-device-required",
        "guest",
        [guard("official input requires the declared transient disk")],
        "layers",
        ["GuestOfficialTransport"],
    ),
    (
        "official-disk-readonly",
        "build_image",
        [("official-readonly",)],
        "core",
        [
            "FakeQemu.test_builder_input_disk_is_readonly_and_not_attached_to_open_assembly",
            "ForwardedInputs.test_provisioning_official_input_disk_is_explicitly_readonly",
        ],
    ),
    (
        "official-http-isolation",
        "build_image",
        [("official-http-isolation",)],
        "core",
        [
            "FakeQemu.test_scratch_builder_http_cannot_serve_nvidia_payloads",
            "FakeQemu.test_open_assembly_http_cannot_serve_nvidia_payloads",
        ],
    ),
    (
        "kernel-source-classification",
        "scan_image",
        [("kernel-source",)],
        "layers",
        ["KernelOssHeaders"],
    ),
    (
        "driver-staging-no-module-load",
        "recipe",
        [("driver-module-load",)],
        "core",
        [
            "GuestInstall.test_driver_staging_does_not_try_to_load_a_gpu_module",
            "CommittedRecipe.test_recipe_declares_gpu_free_driver_staging",
        ],
    ),
]


def contains(node, text):
    return any(
        isinstance(n, ast.Constant) and isinstance(n.value, str) and text in n.value
        for n in ast.walk(node)
    )


def one(nodes):
    if len(nodes) != 1:
        raise ValueError(f"mutation site count={len(nodes)}")
    return nodes[0]


def mutate(source, operations):
    if operations == [("driver-module-load",)]:
        recipe = json.loads(source)
        pin = one(
            [
                p
                for p in recipe["official_fetch"]
                if p["kind"] == "run"
                and p["version"] == recipe["guest"]["nvidia_version"]
            ]
        )
        pin["install_argv"].remove("--skip-module-load")
        return json.dumps(recipe, indent=2) + "\n"
    for operation in operations:
        tree = ast.parse(source)
        kind, *args = operation
        replacement = "False"
        if kind in {"guard", "bool-part"}:
            node = one(
                [
                    n
                    for n in ast.walk(tree)
                    if isinstance(n, ast.If)
                    and any(
                        isinstance(b, ast.Raise) and contains(b, args[0])
                        for b in n.body
                    )
                ]
            ).test
            if kind == "bool-part":
                node = node.values[args[1]]
        elif kind == "finding":
            node = one(
                [
                    n
                    for n in ast.walk(tree)
                    if isinstance(n, ast.Call)
                    and isinstance(n.func, ast.Name)
                    and n.func.id == "finding"
                    and contains(n, args[0])
                ]
            )
            replacement = "None"
        elif kind == "call":
            node = one(
                [
                    n
                    for n in ast.walk(tree)
                    if isinstance(n, ast.Call) and ast.unparse(n) == args[0]
                ]
            )
            replacement = "None"
        elif kind == "check":
            node = one(
                [
                    kw.value
                    for n in ast.walk(tree)
                    if isinstance(n, ast.Call)
                    and ast.unparse(n.func) == "subprocess.run"
                    for kw in n.keywords
                    if kw.arg == "check"
                ]
            )
        elif kind == "argument":
            node = one(
                [
                    n
                    for n in ast.walk(tree)
                    if isinstance(n, ast.Constant) and n.value == args[0]
                ]
            )
            # Change to a valid harmless pip option, so rejection cannot be a CLI error.
            replacement = repr("--disable-pip-version-check")
        elif kind == "kernel-source":
            node = one(
                [
                    n.value
                    for n in ast.walk(tree)
                    if isinstance(n, ast.Assign)
                    and any(
                        isinstance(t, ast.Name) and t.id == "kernel_source"
                        for t in n.targets
                    )
                ]
            )
        elif kind == "official-readonly":
            node = one(
                [
                    n
                    for n in ast.walk(tree)
                    if isinstance(n, ast.Constant)
                    and n.value == ",format=raw,readonly=on"
                ]
            )
            replacement = repr(",format=raw,readonly=off")
        elif kind == "official-http-isolation":
            node = one(
                [
                    n
                    for n in ast.walk(tree)
                    if isinstance(n, ast.Assign)
                    and any(
                        isinstance(t, ast.Name) and t.id == "transient"
                        for t in n.targets
                    )
                ]
            )
            replacement = (
                ast.get_source_segment(source, node)
                + "\n        paths.update(transient)"
            )
        elif kind == "created-cleanup":
            node = one(
                [
                    n.test
                    for n in ast.walk(tree)
                    if isinstance(n, ast.If) and ast.unparse(n.test) == "created"
                ]
            )
            replacement = "True"
        elif kind == "walk-errors":
            node = one(
                [
                    kw.value
                    for n in ast.walk(tree)
                    if isinstance(n, ast.Call) and ast.unparse(n.func) == "os.walk"
                    for kw in n.keywords
                    if kw.arg == "onerror"
                ]
            )
            replacement = "None"
        elif kind == "finding-guard":
            node = one(
                [
                    n.test
                    for n in ast.walk(tree)
                    if isinstance(n, ast.If)
                    and any(
                        isinstance(b, ast.Expr)
                        and isinstance(b.value, ast.Call)
                        and ast.unparse(b.value.func) == "finding"
                        and contains(b, args[0])
                        for b in n.body
                    )
                ]
            )
        elif kind == "forward-rule":
            node = one(
                [
                    n
                    for n in ast.walk(tree)
                    if isinstance(n, ast.Constant)
                    and n.value == ",guestfwd=tcp:10.0.2.100:"
                ]
            )
            replacement = repr(",disabled=tcp:10.0.2.100:")
        elif kind == "nonce":
            node = one(
                [
                    n
                    for n in ast.walk(tree)
                    if isinstance(n, ast.Compare)
                    and ast.unparse(n) == "report.get('nonce') != nonce"
                ]
            )
        elif kind == "builder-links":
            node = one(
                [
                    n.ifs[0]
                    for n in ast.walk(tree)
                    if isinstance(n, ast.comprehension)
                    and n.ifs
                    and "not builder" in ast.unparse(n.ifs[0])
                ]
            )
            replacement = "True"
        elif kind == "opencl-selection":
            node = one(
                [
                    n.test
                    for n in ast.walk(tree)
                    if isinstance(n, ast.If)
                    and ast.unparse(n.test) == "path.name != 'nvidia.icd'"
                ]
            )
        elif kind == "monthly-failed":
            node = one(
                [
                    n.test
                    for n in ast.walk(tree)
                    if isinstance(n, ast.If) and ast.unparse(n.test) == "failed"
                ]
            )
        elif kind == "label":
            node = one(
                [
                    n
                    for n in ast.walk(tree)
                    if isinstance(n, ast.Constant) and n.value == b"cidata"
                ]
            )
            replacement = "b'wrong'"
        else:
            raise ValueError("unknown mutation operation")
        lines = source.splitlines(keepends=True)
        start = sum(map(len, lines[: node.lineno - 1])) + node.col_offset
        end = sum(map(len, lines[: node.end_lineno - 1])) + node.end_col_offset
        source = source[:start] + replacement + source[end:]
        ast.parse(source)
    return source


def failures(output, label, category):
    if category == "ERROR":
        # Loader and setUpClass errors lack the normal Class.method identity.
        # Keep every ERROR header; none may disappear behind a naming pattern.
        return sorted(
            set(
                label + ":" + line
                for line in re.findall(r"^ERROR: (.+)$", output, re.M)
            )
        )
    # Class + method identifies a test uniquely, including when both files have
    # classes with the same name. ERRORs are retained separately and invalidate a run.
    return sorted(
        set(
            label + ":" + cls + "." + name
            for name, cls in re.findall(
                r"^" + category + r": (\w+) \(__main__\.(\w+)\.\w+\)", output, re.M
            )
        )
    )


def run_case(case):
    name, module, operations, label, classes = case
    filename = "recipe.json" if module == "recipe" else module + ".py"
    source = mutate((HERE / filename).read_text(), operations)
    with tempfile.TemporaryDirectory(prefix="apps-image-mutant-") as temporary:
        root = Path(temporary)
        image = root / "scripts/apps/image"
        ci = root / "scripts/ci"
        image.mkdir(parents=True)
        ci.mkdir(parents=True)
        for test in TESTS.values():
            shutil.copyfile(REPO / "scripts/ci" / test, ci / test)
        for file in HERE.iterdir():
            if file.is_file():
                shutil.copyfile(file, image / file.name)
        (image / filename).write_text(source)
        for entry in HERE.parent.iterdir():
            if entry.name != "image":
                (image.parent / entry.name).symlink_to(
                    entry, target_is_directory=entry.is_dir()
                )
        run = subprocess.run(
            [sys.executable, "-I", str(ci / TESTS[label]), *classes],
            cwd=root,
            capture_output=True,
            text=True,
            timeout=300,
        )
        output = run.stdout + run.stderr
        killers = failures(output, label, "FAIL")
        errors = failures(output, label, "ERROR")
        valid = run.returncode == 1 and len(killers) >= 2 and not errors
        return {
            "name": name,
            "killers": killers,
            "count": len(killers),
            "errors": errors,
            "returncode": run.returncode,
            "valid": valid,
            **({} if valid else {"output": output[-16000:]}),
        }


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--jobs", type=int, default=2)
    p.add_argument("--report", type=Path)
    args = p.parse_args()
    if args.jobs not in (1, 2):
        p.error("--jobs must be 1 or 2")
    for test in TESTS.values():
        baseline = subprocess.run(
            [sys.executable, "-I", str(REPO / "scripts/ci" / test)],
            cwd=REPO,
            capture_output=True,
            text=True,
            timeout=300,
        )
        if args.report:
            args.report.parent.mkdir(parents=True, exist_ok=True)
            (args.report.parent / ("baseline-" + Path(test).stem + ".txt")).write_text(
                baseline.stdout + baseline.stderr
            )
        if baseline.returncode:
            print(baseline.stdout + baseline.stderr)
            print("baseline failed; no mutation results are valid")
            return 2
    # Validate every AST site before spending CPU on test subprocesses.
    for _, module, operations, _, _ in CASES:
        filename = "recipe.json" if module == "recipe" else module + ".py"
        mutate((HERE / filename).read_text(), operations)
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        results = []
        for result in pool.map(run_case, CASES):
            results.append(result)
            print(
                ("ok " if result["valid"] else "WEAK ")
                + result["name"]
                + ": "
                + str(result["count"])
                + " assertion killers; "
                + str(len(result["errors"]))
                + " errors",
                flush=True,
            )
    if args.report:
        args.report.write_text(json.dumps(results, indent=2) + "\n")
    passed = sum(r["valid"] for r in results)
    print(
        f"{passed}/{len(results)} mutants killed by at least two named assertion failures"
    )
    return int(passed != len(results))


if __name__ == "__main__":
    sys.exit(main())
