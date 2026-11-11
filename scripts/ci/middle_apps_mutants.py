#!/usr/bin/env python3
"""GPU-free middle-apps policy mutations; baseline green and >=2 named killers each.

Falsifier: a variant survives, an anchor is absent/ambiguous, syntax is invalid,
or a collector fails. Operates only in owned temporary copies; no remote service.
"""

import argparse
import ast
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
BROKER = "ci/vast-broker/"
CASES = [
    (
        "missing-bundle-marker-trusted",
        "scripts/kfci/apps_gate.py",
        "            if (\n                not bundle.is_file()\n                or not bundle.stat().st_size\n                or not marker.is_file()\n            ):",
        "            if not marker.is_file():",
        "py",
    ),
    (
        "missing-bundle-not-media",
        "scripts/kfci/apps_gate.py",
        "RetryClass.MEDIA",
        "RetryClass.NOT_BUILT",
        "py",
    ),
    (
        "llm-payload-slot-enabled",
        "scripts/ci/vast_payload.py",
        '"llm_parity": "DESIGN_ONLY"',
        '"llm_parity": "PLUMBING"',
        "py",
    ),
    (
        "llm-worker-slot-enabled",
        BROKER + "defaults.json",
        '"llm_parity": {\n      "tiers": [\n        "large"\n      ],\n      "state": "DESIGN_ONLY"',
        '"llm_parity": {\n      "tiers": [\n        "large"\n      ],\n      "state": "PLUMBING"',
        "node",
    ),
    (
        "large-loses-middle-inheritance",
        BROKER + "src/tier.js",
        "inherits: 'middle'",
        "inherits: 'small'",
        "node",
    ),
    (
        "base-policy-read-required",
        BROKER + "runner.py",
        '"apps_policy_required": True,',
        '"apps_policy_required": False,',
        "py",
    ),
    (
        "policy-binding-unchecked",
        BROKER + "runner.py",
        "@read_apps_policy",
        'return availability.pop("apps_policy")["enabled"]',
        "py",
    ),
    (
        "linux-request-dropped",
        BROKER + "runner.py",
        'request.update(linuxApps=True, winApps="mid")',
        'request.update(winApps="mid")',
        "py",
    ),
    (
        "windows-request-dropped",
        BROKER + "runner.py",
        'request.update(linuxApps=True, winApps="mid")',
        "request.update(linuxApps=True)",
        "py",
    ),
    (
        "missing-stage-pass",
        BROKER + "runner.py",
        "@complete_requested_apps",
        "return True",
        "py",
    ),
    (
        "combined-middle-pin-bypassed",
        BROKER + "src/offers.js",
        "profile === 'check' && options.tierName === 'middle' && cfg.pinnedMiddleDies.length",
        "false && profile === 'check' && options.tierName === 'middle' && cfg.pinnedMiddleDies.length",
        "node",
    ),
    ("linux-count-weakened", BROKER + "runner.py", '"apps": 66', '"apps": 63', "py"),
    (
        "middle-linux-restores-old-63-probe-subset",
        "scripts/apps/gate_list_middle.txt",
        ["gpu_burn\n", "vkpeak\n", "clpeak\n"],
        ["", "", ""],
        "py",
    ),
    (
        "mixed-app-failure-laundered-as-retry",
        BROKER + "runner.py",
        "@retry_only_failures",
        'return any(r["fail"] and r["name"].startswith(\n'
        '    ("windows_retry_", "apps_retry_", "windows_apps_retry_")\n'
        ") for r in suites)",
        "node",
    ),
    (
        "windows-count-weakened",
        BROKER + "runner.py",
        '"windows_apps": 10',
        '"windows_apps": 9',
        "py",
    ),
    (
        "apps-retry-output-lost",
        BROKER + "runner.py",
        '("windows_retry_", "apps_retry_", "windows_apps_retry_")',
        '("windows_retry_",)',
        "py",
    ),
    (
        "guest-failure-retried",
        BROKER + "runner.py",
        'r["name"].startswith(',
        'True or r["name"].startswith(',
        "py",
    ),
    (
        "linux-apps-spec-row-dropped",
        "scripts/ci/vast_report.py",
        '    ("KF_LINUX_APPS", False, _match(r"1")),\n',
        "",
        "py",
    ),
    (
        "linux-apps-spec-row-dropped-worker-side",
        "scripts/ci/vast_report.py",
        '    ("KF_LINUX_APPS", False, _match(r"1")),\n',
        "",
        "node",
    ),
    (
        "linux-apps-spec-validator-loosened",
        "scripts/ci/vast_report.py",
        '("KF_LINUX_APPS", False, _match(r"1")),',
        '("KF_LINUX_APPS", False, _match(r"1|0|true|yes")),',
        "py",
    ),
    (
        "linux-apps-spec-validator-loosened-worker-side",
        "scripts/ci/vast_report.py",
        '("KF_LINUX_APPS", False, _match(r"1")),',
        '("KF_LINUX_APPS", False, _match(r"1|0|true|yes")),',
        "node",
    ),
    (
        "winapps-spec-row-dropped",
        "scripts/ci/vast_report.py",
        '    ("KF_WINAPPS", False, _match(r"small|mid|full")),\n',
        "",
        "py",
    ),
    (
        "spec-lane-spec-row-dropped-worker-side",
        "scripts/ci/vast_report.py",
        '    ("KF_SPEC", False, _match(r"lanea|laneb|both")),\n',
        "",
        "node",
    ),
    (
        "stages-spec-row-dropped-worker-side",
        "scripts/ci/vast_report.py",
        '    ("KF_STAGES", False, _match(r"[a-z][a-z0-9_]{0,31}(?::[a-z][a-z0-9_]{0,31}){0,3}")),\n',
        "",
        "node",
    ),
    (
        "payload-header-linuxapps-dropped",
        "scripts/ci/vast_payload.py",
        '\n            + " linuxapps="\n            + str(int(linuxapps)),',
        ",",
        "py",
    ),
    (
        "explicit-linux-opt-in-ignored",
        "scripts/ci/vast_payload.py",
        "@linuxapps_requested",
        'environ.pop("KF_LINUX_APPS", None)\nreturn apps_requested(tier)',
        "py",
    ),
    (
        "linux-opt-in-tier-ignored",
        "scripts/ci/vast_payload.py",
        'value != "1" or tier != "middle"',
        'value != "1"',
        "py",
    ),
    (
        "linux-opt-in-value-ignored",
        "scripts/ci/vast_payload.py",
        'value != "1" or tier != "middle"',
        'tier != "middle"',
        "py",
    ),
    (
        "linux-transport-flag-leaks",
        "scripts/ci/vast_payload.py",
        'environ.pop("KF_LINUX_APPS", None)',
        'environ.get("KF_LINUX_APPS", None)',
        "py",
    ),
    (
        "linux-suffix-reserve-lost",
        "scripts/ci/vast_payload.py",
        "APPS_MIN_REMAINING_MS + (",
        "APPS_MIN_REMAINING_MS + 0 * (",
        "py",
    ),
    (
        "gate-suffix-reserve-lost",
        "scripts/ci/vast_payload.py",
        "min_remaining_ms=WINDOWS_MIN_REMAINING_MS + (",
        "min_remaining_ms=WINDOWS_MIN_REMAINING_MS + 0 * (",
        "py",
    ),
    (
        "mid-cleanup-margin-lost",
        "scripts/ci/vast_payload.py",
        '"mid": 70 * 60000',
        '"mid": 60 * 60000',
        "py",
    ),
    (
        "linux-log-late-bound",
        "scripts/ci/vast_payload.py",
        (
            "lambda rc, names=names, output=output_apps: apps_rows",
            "apps_rows(bounded_text(output), rc, names)",
        ),
        (
            "lambda rc, names=names: apps_rows",
            "apps_rows(bounded_text(output_apps), rc, names)",
        ),
        "py",
    ),
    (
        "media-alias-lost",
        "scripts/ci/vast_payload.py",
        'cls = "MEDIA" if cls == "NOT_BUILT" else cls',
        "cls = cls",
        "py",
    ),
    (
        "media-class-refused",
        "scripts/ci/vast_payload.py",
        'APPS_RETRY_CLASSES = {"MEDIA",',
        "APPS_RETRY_CLASSES = {",
        "py",
    ),
    (
        "owner-default-disabled",
        BROKER + "src/policy.js",
        "env.MIDDLE_APPS_ENABLED !== 'false'",
        "env.MIDDLE_APPS_ENABLED === 'true'",
        "node",
    ),
    (
        "owner-switch-ignored",
        BROKER + "src/policy.js",
        "env.MIDDLE_APPS_ENABLED !== 'false'",
        "true",
        "node",
    ),
    (
        "owner-switch-schema-lost",
        BROKER + "src/policy.js",
        "  'MIDDLE_APPS_ENABLED',\n",
        "",
        "node",
    ),
    (
        "signed-tier-ignored",
        BROKER + "src/tier.js",
        "tierName === 'middle' && enabled",
        "enabled",
        "node",
    ),
    (
        "linux-request-policy-unchecked",
        BROKER + "src/index.js",
        "rentRequest.linuxApps !== appRequest.linuxApps",
        "false",
        "node",
    ),
    (
        "windows-request-policy-unchecked",
        BROKER + "src/index.js",
        "rentRequest.winApps !== appRequest.winApps",
        "false",
        "node",
    ),
    (
        "linux-environment-dropped",
        BROKER + "src/tier.js",
        "env.KF_LINUX_APPS = '1';",
        "void linuxApps;",
        "node",
    ),
    (
        "windows-environment-dropped",
        BROKER + "src/tier.js",
        "env.KF_WINAPPS = winApps;",
        "void winApps;",
        "node",
    ),
    (
        "stored-pass-not-checked",
        BROKER + "src/index.js",
        "payload.result = middleAppsResult(payload.result, r.middleAppsRequested === true);",
        "payload.result = payload.result;",
        "node",
    ),
    (
        "stored-count-not-checked",
        BROKER + "src/tier.js",
        "row.name === name && row.pass === count",
        "row.name === name",
        "node",
    ),
    (
        "stored-guest-failure-relabelled",
        BROKER + "src/tier.js",
        "result.verdict !== 'PASS'",
        "false",
        "node",
    ),
    (
        "stored-retry-row-duplicated",
        BROKER + "src/tier.js",
        "row.name !== missing[0] + '_retry_operator'",
        "true",
        "node",
    ),
    (
        "deployed-switch-not-boolean",
        BROKER + "deploy.mjs",
        "  'MIDDLE_APPS_ENABLED',\n]);\nexport const ALLOWED",
        "]);\nexport const ALLOWED",
        "config",
    ),
    (
        "deployed-switch-not-allowed",
        BROKER + "deploy.mjs",
        "  'MIDDLE_APPS_ENABLED',\n  // Tiers",
        "  // Tiers",
        "config",
    ),
    # ---- Linux apps media on a fresh Worker box (2026-11-10): test_apps_media.py and test_apps_pins.py ----------
    # Provisioning flag dropped: the stage must be planned with KF_APPS_PROVISION=1 and honour it.
    (
        "provision-flag-dropped-from-the-stage",
        "scripts/ci/vast_payload.py",
        '                    "env",\n                    "KF_APPS_PROVISION=1",\n                    "bash",\n'
        '                    str(root / "scripts/bench/box/apps_gate.sh"),',
        '                    "bash",\n                    str(root / "scripts/bench/box/apps_gate.sh"),',
        "py",
    ),
    (
        "provision-flag-planned-as-zero",
        "scripts/ci/vast_payload.py",
        '"KF_APPS_PROVISION=1",',
        '"KF_APPS_PROVISION=0",',
        "py",
    ),
    (
        "provision-flag-left-in-the-environment",
        "scripts/ci/vast_payload.py",
        'environ.pop("KF_APPS_PROVISION", None)',
        'environ.get("KF_APPS_PROVISION", None)',
        "py",
    ),
    (
        "provision-flag-ignored-by-the-stage",
        "scripts/kfci/apps_gate.py",
        'if env.get("KF_APPS_PROVISION") == "1":',
        "if False:",
        "py",
    ),
    (
        "apps-reserve-back-to-sixty",
        "scripts/ci/vast_payload.py",
        "APPS_MIN_REMAINING_MS = 100 * 60000",
        "APPS_MIN_REMAINING_MS = 60 * 60000",
        "py",
    ),
    (
        "apps-reserve-below-provisioning-plus-matrix",
        "scripts/ci/vast_payload.py",
        "APPS_MIN_REMAINING_MS = 100 * 60000",
        "APPS_MIN_REMAINING_MS = 70 * 60000",
        "py",
    ),
    # Build failure laundered as an app failure (or as nothing): it must be a MEDIA retry.
    (
        "provision-failure-graded-as-fail",
        "scripts/kfci/apps_gate.py",
        "                    marker.unlink(missing_ok=True)",
        "                    return StepResult(Verdict.FAIL)",
        "py",
    ),
    (
        "provision-log-not-read",
        "scripts/kfci/apps_gate.py",
        "names = provision_failures(logfile)",
        "names = []",
        "py",
    ),
    (
        "guest-setup-failures-not-recognised",
        "scripts/kfci/apps_gate.py",
        "((?:BUILD|SETUP)_[A-Za-z0-9_]{1,64})=FAIL",
        "((?:BUILD)_[A-Za-z0-9_]{1,64})=FAIL",
        "py",
    ),
    (
        "build-failure-after-exit-zero-ignored",
        "scripts/kfci/apps_gate.py",
        '            if self.failed_steps(ctx, logfile, "BUILD_"):\n                return 1\n',
        "            if False:\n                return 1\n",
        "py",
    ),
    (
        "build-script-does-not-count-failures",
        "scripts/apps/build_bundle.sh",
        'echo "BUILD_$1=FAIL rc=$2"; BUILD_FAILS=$((BUILD_FAILS+1)); fi',
        'echo "BUILD_$1=FAIL rc=$2"; fi',
        "py",
    ),
    (
        "build-script-failure-exits-zero",
        "scripts/apps/build_bundle.sh",
        'echo "BUILD_BUNDLE_FAIL count=$BUILD_FAILS"; exit 1; }',
        'echo "BUILD_BUNDLE_FAIL count=$BUILD_FAILS"; exit 0; }',
        "py",
    ),
    # Unpinned or mismatching input accepted.
    (
        "file-fetch-accepts-an-empty-pin",
        "scripts/apps/pin_lib.sh",
        "  pin_is_sha256 \"$want\" || { pin_refuse \"sha256 pin for $url is empty or malformed: '$want'\"; return 1; }\n",
        "",
        "py",
    ),
    (
        "file-verify-accepts-another-digest",
        "scripts/apps/pin_lib.sh",
        "  [ \"$got\" = \"$want\" ] || { pin_refuse \"$path: sha256 $got is not the pinned $want\"; return 1; }\n",
        "",
        "py",
    ),
    (
        "git-fetch-accepts-a-branch-or-empty-pin",
        "scripts/apps/pin_lib.sh",
        "  pin_is_commit \"$want\" || { pin_refuse \"git pin for $url is not a 40-hex commit: '$want'\"; return 1; }\n",
        "",
        "py",
    ),
    (
        "git-fetch-accepts-a-moved-tag",
        "scripts/apps/pin_lib.sh",
        '  if [ "$have" != "$want" ]; then',
        "  if false; then",
        "py",
    ),
    (
        "cuda-toolkit-through-the-apt-shim",
        "scripts/apps/pin_lib.sh",
        "command apt-get -o DPkg::Lock::Timeout=300 -o Acquire::Retries=3 install",
        "apt-get -o DPkg::Lock::Timeout=300 -o Acquire::Retries=3 install",
        "py",
    ),
    (
        "cuda-keyring-unchecked",
        "scripts/apps/pin_lib.sh",
        '  pin_is_sha256 "$want" || { pin_refuse "no pinned CUDA keyring for $rel"; return 1; }\n',
        "",
        "py",
    ),
    (
        "bundle-script-clones-gpu-burn-unpinned",
        "scripts/apps/build_bundle.sh",
        'pin_git_checkout "$S/gpu-burn" "$PIN_GPU_BURN_URL" "" "$PIN_GPU_BURN_COMMIT"; ok src_gpu_burn $?',
        '[ -d "$S/gpu-burn" ] || git clone -q --depth 1 "$PIN_GPU_BURN_URL" "$S/gpu-burn"; ok src_gpu_burn $?',
        "py",
    ),
    (
        "bundle-script-takes-vkpeak-without-its-digest",
        "scripts/apps/build_bundle.sh",
        'pin_fetch_file "$PIN_VKPEAK_URL" "$VKDIR/vkpeak.zip" "$PIN_VKPEAK_SHA256" "$PIN_VKPEAK_SIZE"',
        'curl -fsSL -o "$VKDIR/vkpeak.zip" "$PIN_VKPEAK_URL"',
        "py",
    ),
    (
        "guest-setup-takes-the-model-from-main",
        "scripts/apps/setup_side.sh",
        'pin_fetch_file "$PIN_GGUF_URL" "$G" "$PIN_GGUF_SHA256" "$PIN_GGUF_SIZE"',
        'curl -fsSL -o "$G" "$PIN_GGUF_URL"',
        "py",
    ),
    (
        "model-pin-moves-to-main",
        "scripts/apps/pins.sh",
        "resolve/91cad51170dc346986eccefdc2dd33a9da36ead9/qwen2.5-1.5b-instruct-q4_k_m.gguf",
        "resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
        "py",
    ),
    (
        "model-digest-unpinned",
        "scripts/apps/pins.sh",
        "PIN_GGUF_SHA256=6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
        "PIN_GGUF_SHA256=",
        "py",
    ),
    (
        "keyring-digest-unpinned",
        "scripts/apps/pins.sh",
        "PIN_CUDA_KEYRING_SHA256_ubuntu2204=d93190d50b98ad4699ff40f4f7af50f16a76dac3bb8da1eaaf366d47898ff8df",
        "PIN_CUDA_KEYRING_SHA256_ubuntu2204=",
        "py",
    ),
    # ---- review round (2026-11-10): explicit rows, bounded retries, provisioning cap, disk guard ---------------------
    (
        "imports-row-dropped",
        "scripts/apps/setup_side.sh",
        "ok imports $rc",
        "true",
        "py",
    ),
    (
        "model-row-dropped",
        "scripts/apps/setup_side.sh",
        "ok hf_model $rc",
        "true",
        "py",
    ),
    (
        "model-load-result-ignored",
        "scripts/apps/setup_side.sh",
        "  grep -qx HF_MODEL_CACHED /tmp/setup_hf.out; rc=$?\n",
        "",
        "py",
    ),
    (
        "model-snapshot-skipped",
        "scripts/apps/setup_side.sh",
        'pin_hf_snapshot "$D/hf"; rc=$?',
        "rc=0",
        "py",
    ),
    (
        "setup-exit-status-ignored",
        "scripts/apps/setup_side.sh",
        '\n[ "$SETUP_FAILS" -eq 0 ]',
        "",
        "py",
    ),
    (
        "cuda-libs-row-dropped",
        "scripts/apps/build_bundle.sh",
        "ok cuda_libs $rc",
        "true",
        "py",
    ),
    (
        "cuda-headers-row-dropped",
        "scripts/apps/build_bundle.sh",
        "ok cuda_headers $?",
        "true",
        "py",
    ),
    (
        "file-fetch-not-retried",
        "scripts/apps/pin_lib.sh",
        '  pin_retry pin_fetch_once "$url" "$dest" "$want" "$size" && return 0',
        '  pin_fetch_once "$url" "$dest" "$want" "$size" && return 0',
        "py",
    ),
    (
        "retries-unbounded",
        "scripts/apps/pin_lib.sh",
        '[ "$_pr_n" -ge "$_pr_attempts" ] && return "$_pr_rc"',
        '[ "$_pr_n" -ge 99 ] && return "$_pr_rc"',
        "py",
    ),
    (
        "git-fetch-not-retried",
        "scripts/apps/pin_lib.sh",
        '  if ! pin_retry git -C "$dir" fetch -q --depth 1 origin "$want" 2>/dev/null; then',
        '  if ! git -C "$dir" fetch -q --depth 1 origin "$want" 2>/dev/null; then',
        "py",
    ),
    (
        "pip-torch-not-retried",
        "scripts/apps/setup_side.sh",
        "pin_retry $A/venv/bin/pip -q install --retries 5 --timeout 30 torch==2.6.0",
        "$A/venv/bin/pip -q install --retries 5 --timeout 30 torch==2.6.0",
        "py",
    ),
    (
        "apt-install-without-retries",
        "scripts/apps/pin_lib.sh",
        "command apt-get -o DPkg::Lock::Timeout=300 -o Acquire::Retries=3 install",
        "command apt-get -o DPkg::Lock::Timeout=300 install",
        "py",
    ),
    (
        "apt-update-without-retries",
        "scripts/apps/pin_lib.sh",
        "command apt-get -o DPkg::Lock::Timeout=300 -o Acquire::Retries=3 update",
        "command apt-get -o DPkg::Lock::Timeout=300 update",
        "py",
    ),
    (
        "provisioning-cap-raised",
        "scripts/kfci/apps_gate.py",
        "PROVISION_CAP_S = 45 * 60",
        "PROVISION_CAP_S = 120 * 60",
        "py",
    ),
    (
        "build-not-capped",
        "scripts/kfci/apps_gate.py",
        "                timeout=PROVISION_CAP_S,",
        "                timeout=7200,",
        "py",
    ),
    (
        "guest-image-gets-a-fresh-cap",
        "scripts/kfci/apps_gate.py",
        "                timeout=deadline - time.monotonic(),",
        "                timeout=PROVISION_CAP_S,",
        "py",
    ),
    (
        "guest-image-started-with-no-time-left",
        "scripts/kfci/apps_gate.py",
        "            if deadline - time.monotonic() <= 1:",
        "            if False:",
        "py",
    ),
    (
        "disk-guard-dropped",
        "scripts/kfci/apps_gate.py",
        "if not self.enough_disk(ctx, root, bench):",
        "if False:",
        "py",
    ),
    (
        "disk-minimum-lowered",
        "scripts/kfci/apps_gate.py",
        "APPS_MIN_FREE_GB = 60",
        "APPS_MIN_FREE_GB = 6",
        "py",
    ),
    (
        "disk-guard-reads-only-the-apps-root",
        "scripts/kfci/apps_gate.py",
        "free = min(free_bytes(root), free_bytes(bench))",
        "free = free_bytes(root)",
        "py",
    ),
    (
        "disk-guard-reads-only-the-bench",
        "scripts/kfci/apps_gate.py",
        "free = min(free_bytes(root), free_bytes(bench))",
        "free = free_bytes(bench)",
        "py",
    ),
    (
        "disk-row-silent",
        "scripts/kfci/apps_gate.py",
        'ctx.emit("APPS_PROVISION_DISK free_gb=%d need_gb=%d" % (free // 10**9, need))',
        "pass",
        "py",
    ),
    # ---- round 2 of the review (2026-11-10): knobs, retry guard, icd row, expected rows, bounded copy, qemu, reserve ---
    (
        "budget-clamp-removed",
        "scripts/kfci/apps_gate.py",
        "return value if 0 < seconds <= MATRIX_BUDGET_DEFAULT_S else default",
        "return value",
        "py",
    ),
    (
        "old-free-space-knob-lowers-the-minimum",
        "scripts/kfci/apps_gate.py",
        'override = ctx.env.get("KF_APPS_TEST_MIN_FREE_GB")',
        'override = ctx.env.get("KF_APPS_MIN_FREE_GB")',
        "py",
    ),
    (
        "negative-free-space-minimum-accepted",
        "scripts/kfci/apps_gate.py",
        "        if need < 0:\n            raise ValueError(\"negative free-space minimum\")",
        "        pass",
        "py",
    ),
    (
        "payload-keeps-the-stage-knobs",
        "scripts/ci/vast_payload.py",
        "    for knob in APPS_STAGE_KNOBS:\n        environ.pop(knob, None)",
        "    pass",
        "py",
    ),
    (
        "retry-attempts-unchecked",
        "scripts/apps/pin_lib.sh",
        "  [[ $_pr_attempts =~ ^[1-5]$ ]] || _pr_attempts=3\n",
        "",
        "py",
    ),
    (
        "retry-backoff-unchecked",
        "scripts/apps/pin_lib.sh",
        "    [[ $_pr_w =~ ^[0-9]{1,3}$ ]] || _pr_w=3\n",
        "",
        "py",
    ),
    (
        "icd-row-dropped",
        "scripts/apps/setup_side.sh",
        "; ok icd $?",
        "; true",
        "py",
    ),
    (
        "icd-write-unchecked",
        "scripts/apps/setup_side.sh",
        '&& [ -s "$ICD_DIR/nvidia.icd" ]; ok icd $?',
        "; ok icd 0",
        "py",
    ),
    (
        "expected-rows-ignored",
        "scripts/kfci/apps_gate.py",
        '        names += ["missing_" + row for row in expected_rows(prefix, ctx.root) if row not in present]',
        "        pass",
        "py",
    ),
    (
        "expected-row-dropped-from-the-list",
        "scripts/apps/expected_rows.txt",
        "SETUP_hf_model\n",
        "",
        "py",
    ),
    (
        "guest-script-ignores-the-expected-rows",
        "scripts/apps/provision_guest_apps.sh",
        "done < <(grep -E '^SETUP_' \"$HERE/expected_rows.txt\")",
        "done < /dev/null",
        "py",
    ),
    (
        "image-copy-not-bounded",
        "scripts/kfci/apps_gate.py",
        '["cp", "--", str(bench / "guest.qcow2"), str(image)], timeout=remaining',
        '["cp", "--", str(bench / "guest.qcow2"), str(image)], timeout=7200',
        "py",
    ),
    (
        "partial-image-kept",
        "scripts/kfci/apps_gate.py",
        "                    image.unlink(missing_ok=True)  # never leave a partial image that a rerun would trust\n",
        "",
        "py",
    ),
    (
        "provisioning-qemu-not-stopped",
        "scripts/kfci/apps_gate.py",
        "                stop_provisioning_qemu(bench)\n",
        "                pass\n",
        "py",
    ),
    (
        "any-process-is-stopped-as-qemu",
        "scripts/kfci/apps_gate.py",
        '        if pid <= 1 or not Path("/proc/%d/comm" % pid).read_text().startswith("qemu-system"):',
        "        if pid <= 1:",
        "py",
    ),
    (
        "windows-gate-not-reserved-without-windows-apps",
        "scripts/ci/vast_payload.py",
        "WINDOWS_MIN_REMAINING_MS + (WINAPPS_MIN_REMAINING_MS[winapps] if winapps else 0) if windows else 0",
        "WINDOWS_MIN_REMAINING_MS + WINAPPS_MIN_REMAINING_MS[winapps] if windows and winapps else 0",
        "py",
    ),
    (
        "stage-timeout-back-to-two-hours",
        "scripts/kfci/apps_gate.py",
        "timeout_seconds = PROVISION_CAP_S + MATRIX_BUDGET_DEFAULT_S + 5 * 60",
        "timeout_seconds = 10200",
        "py",
    ),
    (
        "open-packages-missing-from-the-cache-policy",
        "scripts/bench/provision_box.sh",
        "local apps=(cmake unzip libegl-dev libgles-dev ocl-icd-opencl-dev opencl-headers)",
        "local apps=(cmake unzip)",
        "py",
    ),
]
COLLECT = """import io,json,sys,unittest
sys.path.insert(0,'scripts/ci')
s=unittest.defaultTestLoader.loadTestsFromNames(['test_middle_apps','test_apps_media','test_apps_pins'])
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps(dict(tests=r.testsRun,killers=sorted({getattr(t,'test_case',t).id() for t,_ in r.failures+r.errors}),details={t.id():v[-2500:] for t,v in r.failures+r.errors})))
"""


def collect(root, mode):
    if mode == "py":
        for directory in [root / "scripts/ci", root / "ci/vast-broker"]:
            shutil.rmtree(directory / "__pycache__", ignore_errors=True)
        run = subprocess.run(
            [sys.executable, "-c", COLLECT],
            cwd=root,
            capture_output=True,
            text=True,
            timeout=900,
        )
        if run.returncode:
            raise RuntimeError(run.stderr[-2000:])
        return json.loads(run.stdout.splitlines()[-1])
    tests = (
        ["config-booleans.test.js"]
        if mode == "config"
        else ["middle-apps.test.js", "env-contract.test.js"]
    )
    run = subprocess.run(
        [
            "node",
            "--test-reporter=tap",
            "--test",
            *[str(root / BROKER / "test" / name) for name in tests],
        ],
        cwd=root,
        capture_output=True,
        text=True,
        timeout=180,
    )
    match = re.search(r"^# tests (\d+)$", run.stdout, re.M)
    if not match:
        raise RuntimeError(
            "Node collection failed: " + run.stderr[-2000:] + run.stdout[-2000:]
        )
    return dict(
        tests=int(match[1]),
        killers=sorted(set(re.findall(r"^not ok \d+ - (.+)$", run.stdout, re.M))),
        details=run.stdout[-4000:],
    )


def mutate(text, old, new):
    if isinstance(old, str) and old.startswith("@"):
        nodes = [
            n
            for n in ast.parse(text).body
            if isinstance(n, ast.FunctionDef) and n.name == old[1:]
        ]
        if len(nodes) != 1:
            raise RuntimeError("function anchor absent/ambiguous " + old)
        node = nodes[0]
        lines = text.splitlines(keepends=True)
        return (
            "".join(lines[: node.body[0].lineno - 1])
            + "".join("    " + line + "\n" for line in new.splitlines())
            + "".join(lines[node.end_lineno :])
        )
    edits = [(old, new)] if isinstance(old, str) else list(zip(old, new, strict=True))
    for before, after in edits:
        if text.count(before) != 1:
            raise RuntimeError("anchor absent/ambiguous: " + repr(before))
        text = text.replace(before, after)
    return text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("only", nargs="?", default="")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="PM1-middle-mutants-") as directory:
        root = Path(directory)
        ignore = shutil.ignore_patterns("node_modules", ".wrangler", "__pycache__")
        for name in [
            "scripts",
            "ci/vast-broker",
            "ci/vast-reaper",
            "ci/platform-api/migrations",
        ]:
            shutil.copytree(ROOT / name, root / name, ignore=ignore)
        shutil.copyfile(ROOT / "ci/requirements.toml", root / "ci/requirements.toml")
        for relative in (
            "traces/apps_linux_20261107/g1/gate_full.out",
            "traces/apps_linux_20261107/g2/gate_mid.out",
        ):
            target = root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, target)
        (root / BROKER / "node_modules").symlink_to(
            ROOT / BROKER / "node_modules", target_is_directory=True
        )
        baselines = {}
        for mode in ("py", "node", "config"):
            result = collect(root, mode)
            if result["killers"]:
                raise RuntimeError("baseline red " + mode + ": " + repr(result))
            baselines[mode] = result["tests"]
        print("baseline green " + json.dumps(baselines), file=sys.stderr, flush=True)
        rows = []
        for name, filename, old, new, mode in CASES:
            if args.only not in name:
                continue
            path = root / filename
            original = path.read_text()
            changed = mutate(original, old, new)
            if changed == original:
                raise RuntimeError("empty mutant " + name)
            if filename.endswith(".py"):
                ast.parse(changed)
            elif filename.endswith(".json"):
                json.loads(changed)
            path.write_text(changed)
            try:
                if filename.endswith((".js", ".mjs")):
                    checked = subprocess.run(
                        ["node", "--check", str(path)],
                        capture_output=True,
                        text=True,
                        timeout=30,
                    )
                    if checked.returncode:
                        raise RuntimeError(
                            "syntax error " + name + ": " + checked.stderr
                        )
                result = collect(root, mode)
            finally:
                path.write_text(original)
            killers = result["killers"]
            if any("_FailedTest" in name for name in killers):
                raise RuntimeError("import failed " + repr(result))
            rows.append(dict(mutant=name, count=len(killers), killers=killers))
            print(
                name + ": " + str(len(killers)) + " killers",
                file=sys.stderr,
                flush=True,
            )
        print(json.dumps(dict(baselines=baselines, cases=rows), indent=2))
        return int(any(row["count"] < 2 for row in rows))


if __name__ == "__main__":
    raise SystemExit(main())
