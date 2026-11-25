#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for the kfroot tool, library and the rootless wiring of the Windows harness (STATUS: LIVE, 2026-11-15).

Each mutant is one exact-once text replacement in a private tree (scripts/ci/kfroot.py, scripts/lib/kfroot.sh,
scripts/bench/box/windows_gate.sh, windows_apps.sh, scripts/bench/windows/win_vm.sh; 2026-11-20 also scripts/ci/vast_payload.py,
scripts/ci/vast_report.py and scripts/bench/host_xid_watch.sh); each must be killed by at least two independently named failing tests of
scripts/ci/test_kfroot.py and scripts/ci/test_kfroot_payload.py, like the other *_mutants.py harnesses here.
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
    "payload": "scripts/ci/vast_payload.py",
    "report": "scripts/ci/vast_report.py",
    "xid": "scripts/bench/host_xid_watch.sh",
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
    M("bootstrap-gpu-lock-mode", "py", "chmod 0666 {lock}", "chmod 0644 {lock}"),
    M("bootstrap-sudo-log-unreadable", "py", "chmod 0640 {SUDO_LOG}", "chmod 0600 {SUDO_LOG}"),
    M("bootstrap-dmesg-stays-restricted", "py", '["sysctl", "-w", "kernel.dmesg_restrict=0"]', '["sysctl", "-w", "kernel.dmesg_restrict=1"]'),
    M("bootstrap-memlock-limited", "py", "- memlock unlimited", "- memlock 65536"),
    M("bootstrap-required-failure-continues", "py", "        if rc and step.required:", "        if False:"),
    M("bootstrap-dry-run-executes", "py", "        if dry_run:\n            results.append({**step.describe(), \"rc\": None})\n            continue", "        if False:\n            continue"),
    # the library
    M("lib-kfroot-ignores-no-new-privs", "lib", "    if grep -q '^NoNewPrivs:[[:space:]]*1'", "    if false && grep -q '^NoNewPrivs:[[:space:]]*1'"),
    M("lib-kfroot-root-goes-through-sudo", "lib", '    if [ "${KFROOT_EUID:-$(id -u)}" = 0 ]; then\n        kf_record root "$@"', '    if false; then\n        kf_record root "$@"'),
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
    # the rootless payload run (2026-11-20, docs/design/V3_KFROOT_AUDIT.md): the audit as data, the static check, the record, the orchestration
    M("stage-run-as-root", "py", "        self.common_setup(stage)\n        self.prepare(stage, tag)\n        return self.dropped(command)", "        self.common_setup(stage)\n        self.prepare(stage, tag)\n        return command"),
    M("stage-run-as-root-by-flag-forgotten", "payload", "                command = rootless.start(name, command, approved, tag)", "                pass"),
    M("root-stage-runs-without-approval", "py", '        elif policy["as"] == "root_stage" and name not in approved:', "        elif False:"),
    M("unclassified-stage-passes", "py", "        if policy is None:\n            tokens.append(f\"unclassified:{name}\")", "        if policy is None:\n            pass"),
    M("user-stage-approvable-as-root", "py", 'if name not in STAGE_POLICY or STAGE_POLICY[name]["as"] != "root_stage":', "if name not in STAGE_POLICY:"),
    M("lint-reads-no-script", "py", '    if policy["as"] != "user":\n        return []\n    out = []', "    return []\n    out = []"),
    M("lint-flags-kfroot-lines", "py", '        if "kfroot " in code_part or "kf_dmesg" in code_part:\n            continue', "        if False:\n            continue"),
    M("lint-misses-dmesg", "py", "apparmor_parser|nvidia-modprobe|useradd|usermod|groupadd|setcap|swapon|dmesg\")", "apparmor_parser|nvidia-modprobe|useradd|usermod|groupadd|setcap|swapon\")"),
    M("lint-misses-root-guards", "py", "        elif ROOT_GUARD.search(code_part):\n            found.append((n, \"root_guard\"))", "        elif False:\n            pass"),
    M("lint-skips-a-missing-script", "py", '            out.append(f"missing_script:{stage}:{rel}")\n            continue', "            continue"),
    M("payload-not-root-accepted", "py", '        tokens = [] if self.geteuid() == 0 else ["payload_not_root"]', "        tokens = []"),
    M("dropped-stage-keeps-memlock-raise", "py", '([] if self.memlock else ["--no-memlock"])', "[]"),
    M("dropped-stage-keeps-roots-home", "py", '[f"HOME={self.home}", f"USER={self.user}", f"LOGNAME={self.user}", ', "[f\"USER={self.user}\", f\"LOGNAME={self.user}\", "),
    M("dropped-stage-keeps-roots-toolchain", "py", 'if d and not d.startswith("/root/")]', "if d]"),
    M("dropped-command-lost", "py", '*run, *command]', '*run]'),
    M("common-setup-runs-every-step", "py", "run=self.run, only_needed=True, memlock=self.memlock,", "run=self.run, only_needed=False, memlock=self.memlock,"),
    M("common-setup-every-stage", "py", "        if self.common_done:\n            return\n        self.common_done = True", "        self.common_done = True"),
    M("windows-prepare-skipped", "py", '        if "windows_prepare" not in policy["prepare"] or self.prepared:\n            return', "        return"),
    M("windows-prepare-failure-ignored", "py", '        if p.returncode:\n            raise RootlessRefused("step_failed:windows_prepare")', "        pass"),
    M("windows-prepare-runs-in-the-tree", "py", 'KF_KFROOT_RECORD=str(mark), KF_ROOTLESS="off")', 'KF_ROOTLESS="off")'),
    M("toolchain-copy-failure-ignored", "py", '        if p.returncode and required:\n            raise RootlessRefused(f"step_failed:{name}")', "        pass"),
    M("root-stage-not-recorded", "py", '            self.record.step("root_stage", stage, 0, "payload", "approved by name: " + policy["blocker"])\n', ""),
    M("root-stage-leaves-root-files", "py", "        elif how in (\"root_stage\", \"self_drop\"):\n            self.give_back(stage)", "        elif False:\n            pass"),
    M("root-stage-without-safe-directory", "py", '"GIT_CONFIG_KEY_0=safe.directory", ', ""),
    M("checkout-trust-wildcard", "payload", '["git", "-c", "safe.directory=" + root, "-C", root, *args]', '["git", "-c", "safe.directory=*", "-C", root, *args]'),
    M("checkout-trust-dropped", "payload", '["git", "-c", "safe.directory=" + root, "-C", root, *args]', '["git", "-C", root, *args]'),
    M("xid-guard-revision-read-without-trust", "payload", 'revision = checkout_git(root, "rev-parse", "HEAD", stderr=None)', 'revision = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()'),
    M("refused-kfroot-not-read", "py", '        if how == "user":\n            self.record.refusals(output_text, stage)', "        if False:\n            pass"),
    M("refused-kfroot-not-a-violation", "py", '        if self.unaudited or self.refused:\n            return "VIOLATED"', "        if self.unaudited:\n            return \"VIOLATED\""),
    M("unaudited-step-not-a-violation", "py", '        if self.unaudited or self.refused:\n            return "VIOLATED"', "        if self.refused:\n            return \"VIOLATED\""),
    M("approved-root-stage-reads-as-rootless", "py", '        if any(s["as"] == "root_stage" for s in self.stages.values()):\n            return "ROOT_STAGES_APPROVED"', "        if False:\n            return \"ROOT_STAGES_APPROVED\""),
    M("unknown-kfroot-command-audited", "py", "            if name is None:\n                self.unaudited.append(printable(command, 48))\n            else:", "            if name is None:\n                pass\n            else:"),
    M("record-unbounded", "py", "        if len(self.steps) < self.LIMIT:", "        if True:"),
    M("record-unprintable", "py", '    return "".join(c if " " <= c <= "~" else "?" for c in str(text))[:limit]', "    return str(text)[:limit]"),
    M("kfroot-command-classified-by-caller", "py", '    if base[:1] == ["apparmor_parser"]:\n        return "swtpm_apparmor_load"', "    if False:\n        return \"swtpm_apparmor_load\""),
    M("stage-without-an-audit-entry", "py", '    "specviewperf": _p("root_stage", "not_yet_run", blocker="scripts/bench/box/specviewperf_stage (.sh): a blanket \'not root\' refusal and raw dmesg"),\n', ""),
    M("merge-bar-becomes-a-user-stage", "py", '"merge_bar": _p("root_stage", "not_yet_run", blocker="scripts/fastguest/build_fast_guest (.sh): modprobe nbd, qemu-nbd --connect, mount -o ro (host root operations)"),', '"merge_bar": _p("user", "not_yet_run", ("scripts/bench/box/merge_check.sh",)),'),
    M("bootstrap-needed-check-always-on", "py", "    need = (lambda missing: missing) if only_needed else (lambda missing: True)", "    need = lambda missing: True"),
    M("bootstrap-needed-check-skips-everything", "py", "    need = (lambda missing: missing) if only_needed else (lambda missing: True)", "    need = lambda missing: False"),
    M("bootstrap-memlock-flag-ignored", "py", "    if memlock:\n        steps.append(Step(\"memlock_limit\"", "    if True:\n        steps.append(Step(\"memlock_limit\""),
    M("bootstrap-only-needed-not-passed-on", "py", "    for step in plan(user, workspace, dev, proc, chown, audit_log, only_needed, memlock, lock, dmesg_restrict):", "    for step in plan(user, workspace, dev, proc, chown, audit_log, False, True, lock, dmesg_restrict):"),
    M("payload-rootless-result-not-recorded", "payload", '        if rootless is not None:\n            result["rootless"] = rootless.record.to_json()', "        if False:\n            pass"),
    M("payload-refused-setup-still-runs", "payload", '        rc = 125 if (short or refusal) else runner(command, soft, stage["output"], stage["progress"])', '        rc = 125 if short else runner(command, soft, stage["output"], stage["progress"])'),
    M("payload-refused-setup-passes", "payload", '            if refusal:\n                raise EvidenceError("rootless_setup")\n', ""),
    M("payload-bad-mode-is-off", "payload", '    if mode not in ROOTLESS_MODES:\n        print("KF_PAYLOAD_ROOTLESS_REFUSED bad_mode", flush=True)\n        raise EvidenceError("preflight")', "    if mode not in ROOTLESS_MODES:\n        mode = \"off\""),
    M("payload-flag-loses-to-variable", "payload", 'mode = args.rootless if args.rootless is not None else environ.get("KF_PAYLOAD_ROOTLESS", "off")', 'mode = environ.get("KF_PAYLOAD_ROOTLESS", args.rootless or "off")'),
    M("payload-options-without-require-ignored", "payload", '        if args.rootless_root_stage or args.rootless_memlock or args.rootless_user:\n            print("KF_PAYLOAD_ROOTLESS_REFUSED rootless_option_without_require", flush=True)\n            raise EvidenceError("preflight")\n', ""),
    M("payload-root-account-allowed", "payload", '        load_kfroot().check_user(user)\n    except Exception:', "        pass\n    except Exception:"),
    M("payload-preflight-not-run", "payload", "            tokens = rootless.preflight(selected, rl[\"approved\"])", "            tokens = []"),
    M("payload-dry-run-skips-the-refusals", "payload", "            tokens = kr.stage_refusals(chosen, root, rl[\"approved\"])", "            tokens = []"),
    M("payload-kfml-prefetch-as-root", "payload", "        PREFETCH_ALLOWED = rootless is None", "        PREFETCH_ALLOWED = True"),
    M("payload-kfml-prefetch-ignores-the-switch", "payload", "    if not PREFETCH_ALLOWED:\n        return None\n", ""),
    M("payload-provision-dir-is-roots", "payload", "        prov_dir = ROOTLESS_PROV_DIR if rootless else PROV_DIR", "        prov_dir = PROV_DIR"),
    M("payload-rootless-not-handed-to-the-executor", "payload", "rootless=rootless, approved=rl[\"approved\"], tag=tag)", "rootless=None, approved=rl[\"approved\"], tag=tag)"),
    M("payload-approvals-not-handed-to-the-executor", "payload", "rootless=rootless, approved=rl[\"approved\"], tag=tag)", "rootless=rootless, approved=(), tag=tag)"),
    M("payload-plain-run-gets-rootless-kwargs", "payload", "        if rootless is None:\n            return execute_stages(stages, soft, result, destination)", "        if False:\n            return execute_stages(stages, soft, result, destination)"),
    M("output-tail-read-strictly", "payload", "        return first.decode(\"utf-8\", errors=\"replace\")", "        return first.decode(\"utf-8\")"),
    M("output-tail-not-read", "payload", "            if size > head + tail:\n                source.seek(size - tail)\n                first += b\"\\n\" + source.read(tail)", "            if size > head + tail:\n                pass"),
    M("bootstrap-installs-the-launcher-for-the-payload", "py", "                                        install_files=False)", "                                        install_files=True)"),
    M("bootstrap-failure-escapes-the-stage-loop", "py", "        except KfrootError as error:\n            raise RootlessRefused(\"bootstrap:\" + printable(error, 60).replace(\" \", \"_\")) from None", "        except ZeroDivisionError as error:\n            raise"),
    M("environment-snapshot-at-construction", "py", "self._environ = None if environ is None else dict(environ)", "self._environ = dict(os.environ if environ is None else environ)"),
    M("bootstrap-log-and-lock-settings-not-passed-on", "py", "plan(user, workspace, dev, proc, chown, audit_log, only_needed, memlock, lock, dmesg_restrict)", "plan(user, workspace, dev, proc, chown, audit_log, only_needed, memlock)"),
    M("bootstrap-ignores-install-files-flag", "py", "    elif install_files:\n        audit_log = install(user, root, visudo, run)[\"audit_log\"]", "    elif True:\n        audit_log = install(user, root, visudo, run)[\"audit_log\"]"),
    M("payload-rootless-env-is-an-instruction-field", "payload", 'ROOTLESS_ENV = ("KF_PAYLOAD_ROOTLESS", ', 'ROOTLESS_ENV = ("KF_RELEASE_REHEARSAL", "KF_PAYLOAD_ROOTLESS", '),
    M("report-accepts-any-rootless-block", "report", '    if not ok:\n        raise ValueError("rootless")', "    if False:\n        raise ValueError(\"rootless\")"),
    M("report-accepts-a-forged-clean-verdict", "report", '    ok = ok and (value["verdict"] != "ROOTLESS" or not (value["refused"] or value["unaudited"]))\n', ""),
    M("report-drops-the-rootless-key", "report", '            "windows_open_rows",\n            "rootless",\n            "full",', '            "windows_open_rows",\n            "full",'),
    M("record-file-not-written", "lib", "    [ -z \"${KF_KFROOT_RECORD:-}\" ] || printf", "    [ -z \"${KF_KFROOT_RECORD:-}\" ] || true || printf"),
    M("refused-use-not-recorded", "lib", '        kf_record refused "$@"\n', ""),
    M("root-use-not-recorded", "lib", '        kf_record root "$@"\n', ""),
    M("xid-watch-raw-dmesg", "xid", "DMESG_RAW=$(xid_dmesg 2>/dev/null)", "DMESG_RAW=$(dmesg 2>/dev/null)"),
    M("xid-watch-raw-dmesg-in-the-delta", "xid", "xid_lines() { xid_dmesg 2>/dev/null |", "xid_lines() { dmesg 2>/dev/null |"),
    M("xid-watch-kfroot-first", "xid", "xid_dmesg() { dmesg 2>/dev/null || kf_dmesg; }", "xid_dmesg() { kf_dmesg; }"),
    M("xid-watch-no-fallback", "xid", "xid_dmesg() { dmesg 2>/dev/null || kf_dmesg; }", "xid_dmesg() { dmesg; }"),
    M("xid-watch-refusal-not-said", "xid", '  [ "$DMESG_RC" -ne 126 ] || say "KFROOT_REFUSED dmesg:', '  [ "$DMESG_RC" -ne 999 ] || say "KFROOT_REFUSED dmesg:'),
    M("xid-watch-needs-the-library", "xid", "else\n  kf_dmesg() { dmesg; }\nfi", "fi"),
]
COLLECT = """import io,json,unittest
s=unittest.defaultTestLoader.discover('scripts/ci',pattern='test_kfroot*.py',top_level_dir='scripts/ci')
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
    for test in ("test_kfroot.py", "test_kfroot_payload.py"):
        shutil.copy(HERE / test, Path(tree) / "scripts/ci" / test)
    for rel in ("docs", "traces", "scripts/apps", "scripts/fastguest"):
        if not (Path(tree) / rel).exists():
            (Path(tree) / rel).parent.mkdir(parents=True, exist_ok=True)
            (Path(tree) / rel).symlink_to(ROOT / rel)
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
