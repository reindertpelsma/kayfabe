#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for the kfroot tool, library and the rootless wiring of the Windows harness (STATUS: LIVE, 2026-11-15).

Each mutant is one exact-once text replacement in a private tree (scripts/ci/kfroot.py, scripts/lib/kfroot.sh,
scripts/bench/box/windows_gate.sh, windows_apps.sh, scripts/bench/windows/win_vm.sh); each must be killed by at least two
independently named failing tests of scripts/ci/test_kfroot.py, like the other *_mutants.py harnesses here.
Run: python3 scripts/ci/kfroot_mutants.py [--jobs N] [--only name,name]  (exit 0 only if every mutant has >= 2 killers)
"""

import concurrent.futures
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
FILES = {
    "py": "scripts/ci/kfroot.py",
    "lib": "scripts/lib/kfroot.sh",
    "gate": "scripts/bench/box/windows_gate.sh",
    "apps": "scripts/bench/box/windows_apps.sh",
    "vm": "scripts/bench/windows/win_vm.sh",
}
SOURCES = {k: (ROOT / v).read_text() for k, v in FILES.items()}


BROKEN = []


def M(name, key, old, new):
    if SOURCES[key].count(old) != 1:
        BROKEN.append((name, old))
        return name, key, SOURCES[key]
    return name, key, SOURCES[key].replace(old, new)


CASES = [
    # the mechanism (owner design)
    M("launcher-prompts", "py", "LAUNCHER_TEXT = f'#!/bin/bash\\nexec sudo -n -E", "LAUNCHER_TEXT = f'#!/bin/bash\\nexec sudo -E"),
    M("launcher-drops-environment", "py", "LAUNCHER_TEXT = f'#!/bin/bash\\nexec sudo -n -E", "LAUNCHER_TEXT = f'#!/bin/bash\\nexec sudo -n"),
    M("wrapper-evals-a-string", "py", "WRAPPER_TEXT = '#!/bin/bash\\nexec \"$@\"\\n'", "WRAPPER_TEXT = '#!/bin/bash\\neval \"$*\"\\n'"),
    M("sudoers-keeps-secure-path", "py", '        f"Defaults:{user} !secure_path\\n"\n', ""),
    M("sudoers-no-setenv", "py", "NOPASSWD:SETENV: {WRAPPER}", "NOPASSWD: {WRAPPER}"),
    M("sudoers-needs-a-password", "py", "NOPASSWD:SETENV: {WRAPPER}", "SETENV: {WRAPPER}"),
    M("sudoers-allows-all-commands", "py", "NOPASSWD:SETENV: {WRAPPER}", "NOPASSWD:SETENV: ALL"),
    M("sudoers-wrapped-log-lines", "py", "Defaults:{user} loglinelen=0\\n", ""),
    M("sudoers-mode-0644", "py", "(SUDOERS, 0o440)", "(SUDOERS, 0o644)"),
    M("visudo-verdict-ignored", "py", "        if ok:\n            logfile = attempt", "        if True:\n            logfile = attempt"),
    M("visudo-refusal-retried-for-anything", "py", 'if not (attempt and "logfile" in out):', "if not attempt:"),
    M("user-root-allowed", "py", 'or user == "root":', "or False:"),
    M("verify-ignores-mode", "py", "if (st.st_mode & 0o7777) != mode:", "if False:"),
    # the drop
    M("drop-without-no-new-privs", "py", '"--inh-caps=-all", "--ambient-caps=-all", "--bounding-set=-all", "--no-new-privs",', '"--inh-caps=-all", "--ambient-caps=-all", "--bounding-set=-all",'),
    M("drop-keeps-the-bounding-set", "py", '"--inh-caps=-all", "--ambient-caps=-all", "--bounding-set=-all", "--no-new-privs",', '"--inh-caps=-all", "--ambient-caps=-all", "--no-new-privs",'),
    M("drop-accepts-uid-zero", "py", "or n <= 0:", "or n < 0:"),
    M("drop-memlock-forgotten", "py", "    if memlock:\n        words +=", "    if False:\n        words +="),
    M("drop-always-through-kfroot", "py", "    if via_kfroot:\n        words.append", "    if True:\n        words.append"),
    M("drop-loses-ld-variables", "py", "ld = carried(environ or {})", "ld = []"),
    # the sudo audit
    M("audit-any-command-allowed", "py", "return bool(words) and words[0] == WRAPPER", "return bool(words)"),
    M("audit-failures-ignored", "py", 'failure = next((f for f in FAILURES if f in head), "")', 'failure = ""'),
    M("audit-window-ignores-the-mark", "py", "f.seek(0 if size < since else since)", "f.seek(0)"),
    M("audit-missing-log-is-empty", "py", 'raise KfrootError(f"audit_log_missing {log}") from None', 'return ""'),
    M("audit-unparsed-lines-skipped", "py", '            entries.append({"user": "?", "command": raw.split("COMMAND=", 1)[1].strip(), "failure": "unparsed"})\n            continue', "            continue"),
    M("audit-wrapped-lines-not-joined", "py", '        if out and line[:1] in (" ", "\\t"):', "        if False:"),
    M("audit-mark-always-zero", "py", "        return os.stat(log).st_size\n    except FileNotFoundError:\n        return 0", "        return 0\n    except FileNotFoundError:\n        return 0"),
    # the /proc assertion
    M("assert-root-uid-accepted", "py", "elif any(int(u) == 0 for u in uids):", "elif False:"),
    M("assert-no-new-privs-ignored", "py", 'if fields.get("NoNewPrivs") != "1":', "if False:"),
    M("assert-bounding-set-ignored", "py", 'CAP_FIELDS = ("CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb")', 'CAP_FIELDS = ("CapInh", "CapPrm", "CapEff", "CapAmb")'),
    M("assert-effective-set-ignored", "py", 'CAP_FIELDS = ("CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb")', 'CAP_FIELDS = ("CapInh", "CapPrm", "CapBnd", "CapAmb")'),
    M("assert-permitted-set-ignored", "py", 'CAP_FIELDS = ("CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb")', 'CAP_FIELDS = ("CapInh", "CapEff", "CapBnd", "CapAmb")'),
    M("assert-unreadable-field-passes", "py", '            bad.append(f"{name.lower()}_unreadable")\n            continue', "            continue"),
    M("assert-descendants-not-read", "py", "(tree_pids(pids, proc) if with_tree else list(pids))", "list(pids)"),
    M("assert-gone-pid-passes", "py", 'procs, violations = [], [f"pid_gone {p}" for p in gone]', "procs, violations = [], []"),
    M("assert-nothing-read-passes", "py", '    if not procs and not violations:\n        violations.append("no_process_read")', "    pass"),
    M("assert-require-never-fails", "py", 'return 0 if ok or a.mode != "require" else 1', "return 0"),
    M("assert-sudo-window-dropped", "py", "    violations = (tree[\"violations\"] if tree else []) + [f\"sudo {v['command'][:80]}\" for v in (sudo[\"violations\"] if sudo else [])]", "    violations = (tree[\"violations\"] if tree else [])"),
    # the box bootstrap
    M("bootstrap-user-not-in-kvm", "py", "useradd --create-home --shell /bin/bash --groups kvm", "useradd --create-home --shell /bin/bash"),
    M("bootstrap-dev-kvm-group-forgotten", "py", 'Step("dev_kvm_group", ["chgrp", "kvm", str(dev / "kvm")])', 'Step("dev_kvm_group", ["true"])'),
    M("bootstrap-gpu-lock-mode", "py", "chmod 0666 {GPU_LOCK}", "chmod 0644 {GPU_LOCK}"),
    M("bootstrap-sudo-log-unreadable", "py", "chmod 0640 {SUDO_LOG}", "chmod 0600 {SUDO_LOG}"),
    M("bootstrap-dmesg-stays-restricted", "py", "kernel.dmesg_restrict=0", "kernel.dmesg_restrict=1"),
    M("bootstrap-memlock-limited", "py", "- memlock unlimited", "- memlock 65536"),
    M("bootstrap-required-failure-continues", "py", "        if rc and step.required:", "        if False:"),
    M("bootstrap-dry-run-executes", "py", "        if dry_run:\n            results.append({**step.describe(), \"rc\": None})\n            continue", "        if False:\n            continue"),
    # the library
    M("lib-kfroot-ignores-no-new-privs", "lib", "    if grep -q '^NoNewPrivs:[[:space:]]*1'", "    if false && grep -q '^NoNewPrivs:[[:space:]]*1'"),
    M("lib-kfroot-root-goes-through-sudo", "lib", '    if [ "${KFROOT_EUID:-$(id -u)}" = 0 ]; then\n        "$@"', '    if false; then\n        "$@"'),
    M("lib-dmesg-always-direct", "lib", '    if [ "${KFROOT_EUID:-$(id -u)}" = 0 ] || [ "$(cat', '    if true || [ "$(cat'),
    M("lib-dmesg-always-kfroot", "lib", '    if [ "${KFROOT_EUID:-$(id -u)}" = 0 ] || [ "$(cat', '    if false && [ "$(cat'),
    M("lib-record-mode-fails-the-run", "lib", '    if [ "$rc" != 0 ] && [ "$mode" = record ]; then', "    if false; then"),
    M("lib-violation-recorded-as-ok", "lib", '    elif [ "$rc" != 0 ] || ! grep -q \' ok=1 \' <<< "$out"; then\n        ROOTLESS_STATE=violated', '    elif [ "$rc" != 0 ] || ! grep -q \' ok=1 \' <<< "$out"; then\n        ROOTLESS_STATE=ok'),
    M("lib-state-not-sticky", "lib", '    elif [ "$ROOTLESS_STATE" != violated ]; then\n        ROOTLESS_STATE=ok', "    else\n        ROOTLESS_STATE=ok"),
    M("lib-unmarked-require-run-passes", "lib", '        ROOTLESS_STATE=violated\n        ROOTLESS_FINAL_RC=1\n    else', '        ROOTLESS_STATE=ok\n        ROOTLESS_FINAL_RC=0\n    else'),
    M("lib-final-violation-dropped", "lib", '        rootless_check final --sudo-since "$KF_SUDO_LOG_MARK" || ROOTLESS_FINAL_RC=1', '        rootless_check final --sudo-since "$KF_SUDO_LOG_MARK" || true'),
    M("lib-unknown-mode-accepted", "lib", '        *) echo "KFROOT_ASSERT mode=$mode ok=0 label=$label (KF_ROOTLESS must be require, record or off)"; return 1 ;;', "        *) ;;"),
    # the harness
    M("gate-assertion-result-ignored", "gate", "rootless_check preflight --pid $$ || refuse rootless-violated", "rootless_check preflight --pid $$ || true"),
    M("gate-root-refusal-is-back", "gate", "rootless_check preflight --pid $$ || refuse rootless-violated", '[ "$(id -u)" = 0 ] || refuse not-root'),
    M("gate-kvm-only-existence", "gate", "[ -c /dev/kvm ] && [ -r /dev/kvm ] && [ -w /dev/kvm ] || refuse no-/dev/kvm-access", "[ -c /dev/kvm ] || refuse no-/dev/kvm-access"),
    M("gate-apt-without-kfroot", "gate", 'DEBIAN_FRONTEND=noninteractive kfroot "${apt_run[@]}"', 'DEBIAN_FRONTEND=noninteractive "${apt_run[@]}"'),
    M("gate-dmesg-direct", "gate", "    xid=$(kf_dmesg 2> /dev/null | grep -c 'NVRM: Xid' || true)", "    xid=$(dmesg 2> /dev/null | grep -c 'NVRM: Xid' || true)"),
    M("gate-unreadable-dmesg-passes", "gate", "kf_dmesg > /dev/null 2>&1 || refuse dmesg-unreadable", "kf_dmesg > /dev/null 2>&1 || true"),
    M("gate-unwritable-dir-passes", "gate", '    [ -d "$t" ] && [ -w "$t" ] || refuse "not-writable-$d"', "    true"),
    M("gate-rootless-pass-allowed", "gate", 'elif [ "${ROOTLESS_FINAL_RC:-0}" != 0 ]; then', "elif false; then"),
    M("gate-pass-line-without-state", "gate", 'verdict PASS NONE "rootless=$ROOTLESS_STATE install=once', 'verdict PASS NONE "install=once'),
    M("gate-no-final-sudo-audit", "gate", "    teardown_guest\n    rootless_final\n", "    teardown_guest\n"),
    M("gate-qemu-not-asserted", "gate", 'rootless_check boot1 --pid $$ --pid-file "$VM/run/qemu.pid" --pid-file "$VM/run/swtpm.pid" \\\n    || fail_op', 'rootless_check boot1 --pid $$ \\\n    || fail_op'),
    M("gate-prepare-does-not-exit", "gate", '    prepare_root || rc_final=1\n    exit "$rc_final"', "    prepare_root || rc_final=1"),
    M("gate-missing-package-in-nnp-is-provider", "gate", 'if [ "$prc" = 126 ]; then', 'if false; then'),
    M("apps-assertion-result-ignored", "apps", "rootless_check preflight --pid $$ || refuse rootless-violated", "rootless_check preflight --pid $$ || true"),
    M("apps-rootless-pass-allowed", "apps", 'if [ "$KIND" = PASS ] && [ "${ROOTLESS_FINAL_RC:-0}" != 0 ]; then', "if false; then"),
    M("apps-dmesg-direct", "apps", "    xid=$(kf_dmesg 2> /dev/null | grep -c 'NVRM: Xid' || true)", "    xid=$(dmesg 2> /dev/null | grep -c 'NVRM: Xid' || true)"),
    M("apps-no-final-sudo-audit", "apps", "GUEST_STARTED=0\nrootless_final\n", "GUEST_STARTED=0\n"),
    M("vm-apparmor-without-kfroot", "vm", '%s\\n\' "$rule" | kfroot tee -a "$local" > /dev/null\n    kfroot apparmor_parser -r "$prof"', '%s\\n\' "$rule" | tee -a "$local" > /dev/null\n    apparmor_parser -r "$prof"'),
    M("vm-dmesg-direct", "vm", '    dm0=$(kf_dmesg 2>/dev/null | wc -l || echo 0)', '    dm0=$(dmesg 2>/dev/null | wc -l || echo 0)'),
    M("vm-no-prepare-command", "vm", "    prepare) install -d", "    prepare_) install -d"),
]
COLLECT = """import io,json,unittest
s=unittest.defaultTestLoader.discover('scripts/ci',pattern='test_kfroot.py',top_level_dir='scripts/ci')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'skipped':len(r.skipped),'killers':[t.id() for t,_ in r.failures+r.errors]}))
"""


def build(tree, key=None, source=None):
    for k, rel in FILES.items():
        dest = Path(tree) / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(source if k == key else SOURCES[k])
        os.chmod(dest, 0o755)
    (Path(tree) / "scripts/ci").mkdir(parents=True, exist_ok=True)
    shutil.copy(HERE / "test_kfroot.py", Path(tree) / "scripts/ci/test_kfroot.py")
    for rel in ("scripts/bench/windows", "scripts/bench/box", "scripts/bench", "scripts/ci"):
        # the scripts read their siblings; only the five mutable files and the test file are private copies
        where = Path(tree) / rel
        for entry in (ROOT / rel).iterdir():
            if not (where / entry.name).exists():
                (where / entry.name).symlink_to(entry)
    subprocess.run(["chmod", "-R", "a+rX", str(tree)], check=True)  # the real-drop tests run as nobody


def run(key=None, source=None):
    with tempfile.TemporaryDirectory(prefix="kfroot-mutant-") as d:
        build(d, key, source)
        p = subprocess.run([sys.executable, "-W", "ignore", "-c", COLLECT], cwd=d, capture_output=True, text=True, timeout=900,
                           env={**os.environ, "KF_KFROOT_TREE": d})
        assert p.returncode == 0, p.stderr[-400:]
        return json.loads(p.stdout.splitlines()[-1])


def main():
    if BROKEN:
        for name, old in BROKEN:
            print("MUTANT_NOT_APPLICABLE %s: %r (not exactly once)" % (name, old))
        return 2
    jobs = int(sys.argv[sys.argv.index("--jobs") + 1]) if "--jobs" in sys.argv else 1
    base = run()
    assert base["killers"] == [], base
    print("baseline: %d tests, %d skipped, 0 failing" % (base["tests"], base["skipped"]))
    cases = CASES
    if "--only" in sys.argv:  # a subset by name, for the loop of writing a test
        wanted = set(sys.argv[sys.argv.index("--only") + 1].split(","))
        cases = [c for c in CASES if c[0] in wanted]
    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
        results = list(pool.map(lambda c: run(c[1], c[2])["killers"], cases))
    weak = []
    for (name, _k, _s), killers in zip(cases, results):
        print("%-44s killers=%d %s" % (name, len(killers), "" if len(killers) >= 2 else "  <-- WEAK"), flush=True)
        if len(killers) < 2:
            weak.append(name)
    print("MUTANTS total=%d weak=%d" % (len(cases), len(weak)))
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
