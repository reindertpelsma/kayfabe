#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free mutation check of the open Windows rows (STATUS: DESIGN-ONLY, 2026-11-17). Every mutant needs two distinct named failing tests.

Same shape as appmatrix/release_mutants.py: each mutant is a source substitution in a private copy of the tree, the open_rows tests run in that
copy, and the mutant is killed only if at least two different tests fail. Only synthetic files, fakes and the host's C compiler are used; no
GPU, no Windows, no network. The unmodified baseline must pass first. The appmatrix directory is linked read-only except for the mutants that
change it (apps.json, gate_profiles.py, verdict.py, winapps.py), which get a private copy.

    python3 scripts/bench/windows/open_rows/open_rows_mutants.py [--jobs 2] [--only NAME]
"""
import argparse
import concurrent.futures
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent                      # .../scripts/bench/windows/open_rows
WIN = HERE.parent                                           # .../scripts/bench/windows
OR = "open_rows/"
AM = "appmatrix/"

# (name, path relative to scripts/bench/windows, old text (must occur once), new text)
MUTANTS = [
    # ---- pins and anchors
    ("commit-mismatch-accepted", OR + "kfopen_pins.py", "    if actual.strip() != wanted:", "    if False:"),
    ("source-override-ignored", OR + "kfopen_pins.py", "    return over or pin", "    return pin"),
    ("binary-digest-pin-allowed", OR + "kfopen_pins.py",
     "            if BINARY_PIN_KEYS.search(key) or (isinstance(value, str) and SHA256_RX.search(value)):", "            if False:"),
    ("http-tool-url-allowed", OR + "kfopen_pins.py", '            if key in tool and not tool[key].startswith("https://"):', "            if False:"),
    ("fingerprint-validation-dropped", OR + "kfopen_pins.py",
     '    if not fps or not all(re.fullmatch(r"[0-9A-F]{64}", f) for f in fps) or doc["nuget_trust"]["signature_validation_mode"] != "require":', "    if False:"),
    ("checksum-table-prefix-match", OR + "kfopen_pins.py",
     '        if not re.search(r"(?<![A-Za-z0-9_.+-])" + re.escape(asset) + r"(?![A-Za-z0-9_.+-])", line):\n            continue', "        pass"),
    ("checksum-ambiguity-accepted", OR + "kfopen_pins.py",
     '    if len(found) > 1:\n        raise OpenRowError("ANCHOR_AMBIGUOUS %s has %d different sha256 values in the upstream text" % (asset, len(found)))',
     "    if False:\n        pass"),
    ("release-conflict-accepted", OR + "kfopen_pins.py", "    if table and digest and table != digest:", "    if False:"),
    ("release-origin-unchecked", OR + "kfopen_pins.py", '    if not url.startswith(download_prefix) or not url.endswith("/" + asset):', "    if False:"),
    ("authenticode-status-ignored", OR + "kfopen_pins.py", '    if status not in ("Valid", "0"):', "    if False:"),
    ("authenticode-signer-ignored", OR + "kfopen_pins.py",
     '    if not re.search(r"(?:^|, )O=\\"?" + re.escape(signer) + r"\\b", subject) and not re.search(r"(?:^|, )CN=\\"?" + re.escape(signer) + r"\\b", subject):',
     "    if False:"),
    ("pypi-version-unchecked", OR + "kfopen_pins.py", '    if doc.get("info", {}).get("version") != version:', "    if False:"),
    ("pypi-origin-unchecked", OR + "kfopen_pins.py", '    if not w["url"].startswith("https://files.pythonhosted.org/"):', "    if False:"),
    ("pypi-ambiguous-wheels-accepted", OR + "kfopen_pins.py", "    if len(wheels) != 1:", "    if not wheels:"),
    ("nuget-signature-mode-weakened", OR + "kfopen_pins.py",
     '% (q(trust["signature_validation_mode"]), q(trust["source"])', '% (q("accept"), q(trust["source"])'),
    ("nuget-fingerprints-dropped", OR + "kfopen_pins.py",
     """    certs = "\\n".join('      <certificate fingerprint=%s hashAlgorithm="SHA256" allowUntrustedRoot="false" />' % q(f) for f in trust["fingerprints_sha256"])""",
     '    certs = ""'),
    # ---- build and sources
    ("anchor-lifted-by-default", OR + "kfopen_build.py",
     '    if ctx.trust.accept_unanchored:\n        return "lifted: %s" % ctx.trust.accept_unanchored\n    verify()', '    return "lifted: default"'),
    ("user-tool-hash-not-recorded", OR + "kfopen_build.py", '    sha = sha256_file(path) if os.path.isfile(path) else "directory"', '    sha = "unrecorded"'),
    ("zip-traversal-allowed", OR + "kfopen_build.py", "            if target != base and not target.startswith(base + os.sep):", "            if False:"),
    ("dirty-source-allowed", OR + "kfopen_build.py", "    if dirty:\n        raise OpenRowError(\"SOURCE_DIRTY", "    if False:\n        raise OpenRowError(\"SOURCE_DIRTY"),
    ("clone-head-unverified", OR + "kfopen_build.py",
     '        P.check_commit(name, git(ctx, git_exe, ["rev-parse", "HEAD"], cwd=d, timeout=60).out, commit)', "        pass"),
    ("cmake-args-dropped", OR + "kfopen_build.py", '+ list(b["cmake_args"]) +', "+ [] +"),
    ("msvc-reboot-code-is-failure", OR + "kfopen_build.py", '    if r.rc not in spec["ok_exit_codes"]:', "    if r.rc != 0:"),
    ("nuget-failure-ignored", OR + "kfopen_build.py", '    if r.rc != 0:\n        raise OpenRowError("NUGET_FAILED', '    if False:\n        raise OpenRowError("NUGET_FAILED'),
    ("recipe-hash-ignores-commit", OR + "kfopen_build.py", '    body = {"commit": pins["sources"]["cts" if key.startswith("cts") else "dxs"]["commit"], ', '    body = {"commit": "x", '),
    ("trust-record-always-git-only", OR + "kfopen_common.py",
     '"mode": "git-only" if not (self.accept_unanchored or self.commit_overrides or self.user_tools or self.prebuilt) else "lifted"', '"mode": "git-only"'),
    ("runner-accepts-shell-strings", OR + "kfopen_common.py", "        if not isinstance(argv, (list, tuple)) or not all(isinstance(a, str) for a in argv):", "        if False:"),
    ("scrub-uuids-is-a-noop", OR + "kfopen_common.py", "    return UUID_RX.sub(sub, text)", "    return text"),
    # ---- caselists
    ("select-takes-everything", OR + "cts_caselist.py", '    if take == "all" or take >= len(pool):\n        return pool', "    return pool"),
    ("caselist-duplicates-allowed", OR + "cts_caselist.py", "    dups = sorted({c for c in cases if cases.count(c) > 1}) if len(cases) < 5000 else []", "    dups = []"),
    ("caselist-hash-not-compared", OR + "cts_caselist.py", '        if sha256_text(raw) != lst.get("sha256"):', "        if False:"),
    ("caselist-root-not-checked", OR + "cts_caselist.py", "        if not c.startswith(ROOTS[name]):", "        if False:"),
    # ---- results, diff, runner
    ("qpa-terminate-ignored", OR + "cts_results.py", "        t = TERM_RX.match(line)\n        if t:", "        t = None\n        if t:"),
    ("qpa-open-case-lost", OR + "cts_results.py", "    return cases, cur", "    return cases, None"),
    ("feature-loss-not-classified", OR + "cts_results.py",
     '    if native in OKISH and guest == "NotSupported":\n        return "GUEST_LOSES_FEATURE"\n', ""),
    ("missing-cases-ignored", OR + "cts_results.py", '    if only_n or only_g:\n        verdict = "INCOMPLETE"', '    if False:\n        verdict = "INCOMPLETE"'),
    ("differences-ignored", OR + "cts_results.py", '    elif rows:\n        verdict = "FAIL"', '    elif False:\n        verdict = "FAIL"'),
    ("duplicate-results-accepted", OR + "cts_results.py",
     '        if name in out:\n            raise OpenRowError("RESULTS_DUPLICATE %s" % name)', '        if False:\n            raise OpenRowError("RESULTS_DUPLICATE %s" % name)'),
    ("details-not-scrubbed", OR + "cts_results.py", "scrub_uuids(d).replace", "d.replace"),
    ("crash-not-resumed", OR + "cts_run.py", "        queue = missing + rest", "        queue = rest"),
    ("timeout-booked-as-crash", OR + "cts_run.py", '"Timeout" if res.timed_out else "Crash"', '"Crash"'),
    ("control-identity-ignored", OR + "cts_run.py",
     '    for key in ("cts_commit", "caselist_sha256", "binary"):\n        if header.get(key) != control_header.get(key):',
     '    for key in ("cts_commit", "caselist_sha256", "binary"):\n        if False:'),
    ("control-gpu-ignored", OR + "cts_run.py",
     '    if not allow_different_gpu and control_header.get("gpu_name") not in (gpu_name, None, "unknown") and gpu_name != "unknown":', "    if False:"),
    ("rerun-adopts-any-result", OR + "cts_run.py", "        if again[c][0] == control[c]:", "        if True:"),
    ("no-control-is-not-notrun", OR + "cts_run.py", '    if control is None:\n        print("KFNOTRUN', '    if False:\n        print("KFNOTRUN'),
    # ---- sample harness
    ("blank-window-accepted", OR + "dxs_harness.py", '"nonblank": len(seen) >= MIN_UNIQUE_COLOURS', '"nonblank": True'),
    ("frames-advance-ignored", OR + "dxs_harness.py", '    if spec.get("frames_advance") and len(caps) >= 2:', "    if False:"),
    ("exit-code-unchecked", OR + "dxs_harness.py", '    checks.append(("exit_code_zero", close.get("rc") == 0,', '    checks.append(("exit_code_zero", True,'),
    ("close-not-required", OR + "dxs_harness.py", '    checks.append(("close_graceful", bool(close.get("exited")),', '    checks.append(("close_graceful", True,'),
    ("title-sequence-ignored", OR + "dxs_harness.py", '            checks.append(("title_sequence", got == want,', '            checks.append(("title_sequence", True,'),
    ("early-exit-passes", OR + "dxs_harness.py",
     '    checks.append(("sample_started_window", bool(facts.get("window_found")) and early is None,', '    checks.append(("sample_started_window", bool(facts.get("window_found")),'),
    ("mip-model-no-clamp", OR + "dxs_harness.py", "            cur = max(cur - 1, 0)", "            cur = cur - 1"),
    ("key-code-changed", OR + "dxs_harness.py", 'VK = {"SPACE": 0x20,', 'VK = {"SPACE": 0x21,'),
    ("capture-shape-unchecked", OR + "dxs_harness.py", "    if width < MIN_WINDOW_PX or height < MIN_WINDOW_PX or len(bgra) != width * height * 4:", "    if False:"),
    ("uav-count-ignored", OR + "dxs_harness.py", '("uav_lines", len(uav) == spec["uav_lines"],', '("uav_lines", True,'),
    ("uav-zero-accepted", OR + "dxs_harness.py", '("uav_written", any(v for _, v in uav),', '("uav_written", True,'),
    ("work-graphs-unsupported-not-notrun", OR + "dxs_harness.py",
     '    if spec.get("not_supported_regex") and re.search(spec["not_supported_regex"], text):\n        return [], "notrun", []',
     '    if False:\n        return [], "notrun", []'),
    ("console-control-ignored", OR + "dxs_harness.py", '    if control is not None:\n        checks.append(("equals_control"', '    if False:\n        checks.append(("equals_control"'),
    # ---- soak
    ("soak-duration-ignored", OR + "gl_soak.py", '        ("soak_duration", secs >= want - 5,', '        ("soak_duration", True,'),
    ("nvoglv-crash-ignored", OR + "gl_soak.py", '        ("no_nvoglv_crash", not ev["crash_in_driver"],', '        ("no_nvoglv_crash", True,'),
    ("tdr-ignored", OR + "gl_soak.py", '        ("no_tdr", not ev["tdr"],', '        ("no_tdr", True,'),
    ("app-crash-ignored", OR + "gl_soak.py", '        ("no_app_crash", not ev["app_crash"],', '        ("no_app_crash", True,'),
    ("soak-result-line-ignored", OR + "gl_soak.py", '(f["result"] or "").startswith("OK")', "True"),
    ("renderer-not-checked", OR + "gl_soak.py", 'bool(f["renderer"]) and "NVIDIA" in f["renderer"]', "True"),
    ("event-query-failure-passes", OR + "gl_soak.py", "    ok = all(c[1] for c in checks) and events_ok", "    ok = all(c[1] for c in checks)"),
    # ---- the LibreHardwareMonitor row (hwmon_row.py, the release-zip anchor)
    ("hwmon-nvidia-hardware-not-required", OR + "hwmon_row.py", '("lhm_nvidia_hardware", bool(gpus),', '("lhm_nvidia_hardware", True,'),
    ("hwmon-gpu-type-ignored", OR + "hwmon_row.py", 'if h.get("type") == hw_type and NVIDIA_RX.search', 'if NVIDIA_RX.search'),
    ("hwmon-gpu-name-ignored", OR + "hwmon_row.py", ' and NVIDIA_RX.search(str(h.get("name", "")))]', ']'),
    ("hwmon-required-types-dropped", OR + "hwmon_row.py", '    for t in row["required_sensor_types"]:', "    for t in []:"),
    ("hwmon-valueless-sensor-counts", OR + "hwmon_row.py",
     '        vals = [s["value"] for s in sensors if s.get("type") == t and numeric(s.get("value"))]', '        vals = [s for s in sensors if s.get("type") == t]'),
    ("hwmon-temperature-window-ignored", OR + "hwmon_row.py", '    checks.append(("lhm_temperature_plausible", all(lo <= v <= hi for v in temps),', '    checks.append(("lhm_temperature_plausible", True,'),
    ("hwmon-load-window-ignored", OR + "hwmon_row.py", '    checks.append(("lhm_load_plausible", all(llo <= v <= lhi for v in loads),', '    checks.append(("lhm_load_plausible", True,'),
    ("hwmon-zero-clock-accepted", OR + "hwmon_row.py", '    checks.append(("lhm_clock_positive", all(v > 0 for v in clocks),', '    checks.append(("lhm_clock_positive", True,'),
    ("hwmon-zero-memory-accepted", OR + "hwmon_row.py", '    checks.append(("lhm_memory_total_positive", all(v > 0 for v in mem),', '    checks.append(("lhm_memory_total_positive", True,'),
    ("hwmon-smi-temperature-ignored", OR + "hwmon_row.py", '        checks.append(("lhm_agrees_with_smi_temperature", d <= tol["temperature_c"],', '        checks.append(("lhm_agrees_with_smi_temperature", True,'),
    ("hwmon-smi-memory-ignored", OR + "hwmon_row.py", '        checks.append(("lhm_agrees_with_smi_memory", frac <= tol["memory_fraction"],', '        checks.append(("lhm_agrees_with_smi_memory", True,'),
    ("hwmon-reader-errors-ignored", OR + "hwmon_row.py", '("lhm_no_reader_errors", not as_list(doc.get("errors")),', '("lhm_no_reader_errors", True,'),
    ("hwmon-reader-exit-status-ignored", OR + "hwmon_row.py", "        if res.rc != 0:\n            raise OpenRowError(\"DUMP_FAILED", "        if False:\n            raise OpenRowError(\"DUMP_FAILED"),
    ("hwmon-negctl-gpu-still-enabled", OR + "hwmon_row.py", '"-GpuEnabled", "0" if negctl else "1"]', '"-GpuEnabled", "1"]'),
    ("hwmon-negctl-exit-inverted", OR + "hwmon_row.py", "        return 0 if failed else 1\n    say(\"VERDICT\"", "        return 1 if failed else 0\n    say(\"VERDICT\""),
    ("hwmon-failures-do-not-fail-the-row", OR + "hwmon_row.py", '    failed = [c[0] for c in checks if not c[1]]', "    failed = []"),
    ("lhm-zip-hash-unchecked", OR + "kfopen_build.py", '    status = anchored(ctx, "lhm", lambda: P.verify_sha256(got, expected))', '    status = "verified"'),
    ("lhm-release-prefix-unpinned", OR + "kfopen_build.py",
     'P.expected_from_github_release(rel, spec["asset"], spec["download_prefix"])\n    zp = ctx.p("downloads", spec["asset"])\n    got = ctx.fetcher.download(url, zp)\n    status = anchored(ctx, "lhm"',
     'P.expected_from_github_release(rel, spec["asset"], "")\n    zp = ctx.p("downloads", spec["asset"])\n    got = ctx.fetcher.download(url, zp)\n    status = anchored(ctx, "lhm"'),
    ("lhm-zip-without-library-accepted", OR + "kfopen_build.py", '    if not os.path.exists(os.path.join(dest, "LibreHardwareMonitorLib.dll")):\n        raise OpenRowError("INSTALL_FAILED the LibreHardwareMonitor',
     '    if False:\n        raise OpenRowError("INSTALL_FAILED the LibreHardwareMonitor'),
    ("hwmon-row-needs-no-tool", OR + "kfopen_build.py", '"hwmon": ("lhm",), ', '"hwmon": (), '),
    ("hwmon-offline-guest-runs-anyway", OR + "kfopen.py", '        if not ctx.trust.prebuilt and not os.path.isdir(ctx.p("tools", "lhm")) and not can_reach():', "        if False:"),
    # ---- command line
    ("offline-guest-runs-anyway", OR + "kfopen.py",
     '    if not ctx.trust.prebuilt and not os.path.isdir(ctx.p("build")) and not os.path.isdir(ctx.p("src")) and not can_reach():', "    if False:"),
    ("not-run-row-runs", OR + "kfopen.py", '        return "%s (%s)" % (row["reason"], row["detail"])', "        return None"),
    # ---- C logic of kf_gldual
    ("c-expected-sum-wrong", OR + "kf_gldual_logic.h", "    uint64_t tri = ((uint64_t)n * (uint64_t)(n - 1u)) / 2u;", "    uint64_t tri = ((uint64_t)n * (uint64_t)(n - 1u)) / 3u;"),
    ("c-result-ignores-bad", OR + "kf_gldual_logic.h", "    if (a->bad || b->bad) {", "    if (0) {"),
    ("c-result-ignores-idle-thread", OR + "kf_gldual_logic.h", "    if (a->iters == 0 || b->iters == 0) {", "    if (0) {"),
    # ---- the matrix side
    ("online-rows-leak-into-full", AM + "gate_profiles.py",
     '    non_gating = set(prof["manual"]["apps"]) | set(prof.get("known_unsupported", {})) | online_source(prof)',
     '    non_gating = set(prof["manual"]["apps"]) | set(prof.get("known_unsupported", {}))'),
    ("verdict-kfnotrun-unknown", AM + "verdict.py", 'START_FAILED|^KFNOTRUN ", re.M)', 'START_FAILED", re.M)'),
    ("verdict-proof-all-ignored", AM + "verdict.py",
     "    return ((not missed) if require_all else (bool(held) or not proofs)), held, missed", "    return (bool(held) or not proofs), held, missed"),
    ("apps-json-nbody-loses-proof-all", AM + "apps.json",
     '   "proof_all": true,\n   "cmd": "kfopen.py run dxsamples_nbody_async', '   "cmd": "kfopen.py run dxsamples_nbody_async'),
    ("apps-json-execute-indirect-loses-proof-all", AM + "apps.json",
     '   "proof_all": true,\n   "cmd": "kfopen.py run dxsamples_execute_indirect', '   "cmd": "kfopen.py run dxsamples_execute_indirect'),
    ("push-list-includes-tests", AM + "winapps.py", '        dirs[:] = sorted(d for d in dirs if d not in ("tests", "__pycache__"))', '        dirs[:] = sorted(d for d in dirs if d not in ("__pycache__",))'),
    ("push-list-includes-mutants", AM + "winapps.py", 'if f.endswith("_mutants.py") or ', "if "),
]

COLLECT = '''import io, json, sys, unittest
sys.path.insert(0, "tests")
suite = unittest.defaultTestLoader.discover("tests", pattern="test_*.py")
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print(json.dumps({"tests": r.testsRun, "killers": [t.id() for t, _ in r.failures + r.errors]}))
'''
APPMATRIX_COPY_IGNORE = shutil.ignore_patterns("__pycache__", "tests", "tools", "native", "*.pyc")


def fixture(parent, name, mutate_appmatrix):
    root = Path(parent) / name / "scripts" / "bench" / "windows"
    root.mkdir(parents=True)
    shutil.copytree(HERE, root / "open_rows", ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
    if mutate_appmatrix:
        shutil.copytree(WIN / "appmatrix", root / "appmatrix", ignore=APPMATRIX_COPY_IGNORE)
    else:
        (root / "appmatrix").symlink_to(WIN / "appmatrix", target_is_directory=True)
    # the repo-level files that appmatrix modules read when they are imported or hashed
    for f in ("LICENSE", "LICENSE-APACHE", "LICENSE-GPL", "THIRD_PARTY_NOTICES.md"):
        src = WIN.parents[2] / f
        if src.exists():
            shutil.copy(src, root.parents[2] / f)
    return root


def run(root):
    p = subprocess.run([sys.executable, "-W", "ignore", "-c", COLLECT], cwd=root / "open_rows", capture_output=True, text=True, timeout=600)
    if p.returncode:
        raise RuntimeError("test harness failed: " + p.stderr[-800:])
    return json.loads(p.stdout.splitlines()[-1])


def one(parent, case):
    name, relative, old, new = case
    root = fixture(parent, name, relative.startswith(AM))
    path = root / relative
    text = path.read_text(encoding="utf-8")
    if text.count(old) != 1:
        raise RuntimeError("%s: mutation anchor occurs %d times in %s" % (name, text.count(old), relative))
    path.write_text(text.replace(old, new), encoding="utf-8")
    result = run(root)
    shutil.rmtree(root.parents[2], ignore_errors=True)
    killers = sorted(set(result["killers"]))
    print("%s: %d killers" % (name, len(killers)), file=sys.stderr, flush=True)
    return {"mutant": name, "count": len(killers), "killers": killers}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--only", help="run one mutant by name")
    args = ap.parse_args()
    cases = [m for m in MUTANTS if not args.only or m[0] == args.only]
    names = [m[0] for m in MUTANTS]
    if len(names) != len(set(names)):
        raise RuntimeError("duplicate mutant name")
    with tempfile.TemporaryDirectory(prefix="kf-open-rows-mutants-") as parent:
        baseline = run(fixture(parent, "baseline", True))
        shutil.rmtree(Path(parent) / "baseline", ignore_errors=True)
        if baseline["killers"]:
            raise RuntimeError("BASELINE RED: " + str(baseline["killers"][:5]))
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
            rows = list(pool.map(lambda case: one(parent, case), cases))
    print(json.dumps({"tests": baseline["tests"], "mutants": len(rows), "cases": rows}, indent=1))
    weak = [r["mutant"] for r in rows if r["count"] < 2]
    if weak:
        print("WEAK (fewer than two killers): " + ", ".join(weak), file=sys.stderr)
    return int(bool(weak))


if __name__ == "__main__":
    sys.exit(main())
