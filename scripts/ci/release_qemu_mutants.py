#!/usr/bin/env python3
"""Mutation checks of the QEMU release kit and its workflow: every mutant needs two named killers. STATUS: LIVE, 2026-11-21.

Each mutant is one exact text replacement in a private copy of scripts/release/build_qemu.py or of
.github/workflows/release-qemu.yml; scripts/ci/test_release_qemu.py runs against the copy and the failing test names are the
killers. The baseline (the unmutated copy) must be green. The sandbox holds real copies of the files under test and symlinks to the
rest of the repository (read only), so a mutant never writes outside its own temporary tree.

    python3 scripts/ci/release_qemu_mutants.py [--jobs N] [substring-of-mutant-name]
"""

import argparse
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
FILES = {"kit": "scripts/release/build_qemu.py", "wf": ".github/workflows/release-qemu.yml", "doc": "docs/howto/building.md", "wrap": "scripts/bench/build_kf3_patched.py", "sh": "scripts/bench/build_kf3.sh"}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITE = "test_release_qemu"
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromName(sys.argv[1])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id().split('.', 1)[1] for t, _ in r.failures + r.errors]}))
"""
REAL_COPIES = ("scripts/release/qemu-release.pin", "scripts/release/qemu-release-keys.asc", "scripts/ci/test_release_qemu.py")


def M(name, key, old, new):
    return (name, key, old, new)


MUTANTS = [
    # configure line
    M("configure-gui-enabled", "kit", '"--disable-gtk", "--disable-sdl", "--disable-curses"', '"--enable-gtk", "--disable-sdl", "--disable-curses"'),
    M("configure-may-download", "kit", '"--disable-download", ', ""),
    M("configure-not-stripped", "kit", ', "--enable-strip",', ","),
    M("configure-debug-info-kept", "kit", '"--disable-debug-info", ', ""),
    M("configure-second-target", "kit", '"--target-list=x86_64-softmmu", ', '"--target-list=x86_64-softmmu,aarch64-softmmu", '),
    M("build-path-not-mapped", "kit", '(str(src), MAP_SRC), (str(build), MAP_BUILD), ', '(str(src), MAP_SRC), '),
    M("kayfabe-path-not-mapped", "kit", '(str(kroot), MAP_KAYFABE), (os.path.relpath', '(os.path.relpath'),
    M("pkgversion-fixed", "kit", '"--with-pkgversion=" + pkgversion', '"--with-pkgversion=kayfabe"'),
    M("recorded-line-keeps-real-paths", "kit", '    for real, name in sorted(((str(src), "<qemu-src>"), (str(build), "<qemu-build>"), (str(kroot), "<kayfabe-src>")), key=lambda p: -len(p[0])):\n        text = text.replace(real, name)\n', ""),
    # the release binary has no KF3_NEGCTL_* switch (the feature stays off in the kit and in a plain build_kf3.sh; only --negctl turns it on)
    M("kit-cargo-enables-negctl", "kit", '"--locked", "-p", "kf-qemu")', '"--locked", "-p", "kf-qemu", "--features", "negctl")'),
    M("kit-cargo-enables-all-features", "kit", '"--locked", "-p", "kf-qemu")', '"--locked", "-p", "kf-qemu", "--all-features")'),
    M("plain-build-compiles-the-negctl-feature-in", "sh", "FEATURES=()\nARGS=()", "FEATURES=(--features negctl)\nARGS=()"),
    M("negctl-option-does-not-build-the-feature", "sh", "--negctl) FEATURES=(--features negctl) ;;", "--negctl) ;;"),
    M("unknown-build-option-accepted", "sh", '-*) echo "⊘ unknown option $a (usage: build_kf3.sh [--negctl] <qemu-source-tree> [<build-dir>])" >&2; exit 2 ;;', '-*) ARGS+=("$a") ;;'),
    M("patched-build-asks-for-the-negctl-feature", "wrap", '"build_kf3.sh"), str(copy), str(build)]', '"build_kf3.sh"), "--negctl", str(copy), str(build)]'),
    # versions and inputs
    M("version-check-dropped", "kit", "    if version not in SUPPORTED_QEMU:", "    if False:"),
    M("version-check-by-prefix", "kit", "    if version not in SUPPORTED_QEMU:", "    if not any(version.startswith(v) for v in SUPPORTED_QEMU):"),
    M("version-file-trailing-lines-allowed", "kit", "    if len(lines) != 1 or not re.fullmatch", "    if not lines or not re.fullmatch"),
    M("tree-digest-ignores-exec-bit", "kit", '"f %d %s %s" % (1 if path.stat().st_mode & 0o111 else 0, sha256_file(path), rel)', '"f %d %s %s" % (0, sha256_file(path), rel)'),
    M("tree-digest-ignores-paths", "kit", 'sha256_file(path), rel))', 'sha256_file(path), "x"))'),
    M("tree-digest-reads-git-dir", "kit", 'dirnames[:] = sorted(d for d in dirnames if d != ".git")\n        for name in sorted(filenames + [d for d in dirnames if (Path(dirpath) / d).is_symlink()]):\n            if name == ".git":\n                continue\n            path = Path(dirpath) / name\n            rel = path.relative_to(root).as_posix()\n            if path.is_symlink():\n                lines.append',
      'dirnames[:] = sorted(dirnames)\n        for name in sorted(filenames + [d for d in dirnames if (Path(dirpath) / d).is_symlink()]):\n            path = Path(dirpath) / name\n            rel = path.relative_to(root).as_posix()\n            if path.is_symlink():\n                lines.append'),
    M("tree-copy-keeps-git-dir", "kit", "ignore=shutil.ignore_patterns(\".git\"))", "ignore=None)"),
    # overlay
    M("overlay-kconfig-not-appended", "kit", '        kconfig.write_text(text + ("" if text.endswith("\\n") else "\\n") + KCONFIG_STANZA)', "        pass"),
    M("overlay-meson-appended-twice", "kit", "    if MESON_HUNK not in text:", "    if True:"),
    M("overlay-over-foreign-files", "kit", "    if dest.is_dir() and any(p.name not in ours for p in dest.iterdir()):", "    if False:"),
    M("overlay-missing-archive-allowed", "kit", "    if not archive.is_file():\n        raise KitRefused", "    if False:\n        raise KitRefused"),
    # the optional patch (hyperv-wake): off by default, on when asked, once, in registry order, only on text it fits
    M("optional-patch-applied-by-default-in-a-build", "kit", "    optional = patch_names(with_patches)\n    work = _prepare_workdir(workdir)", "    optional = patch_names(list(with_patches) + [\"hyperv-wake\"])\n    work = _prepare_workdir(workdir)"),
    M("optional-patch-applied-by-default-in-the-overlay", "kit", "    names = patch_names(with_patches)\n    patched = ", "    names = patch_names(list(with_patches) + [\"hyperv-wake\"])\n    patched = "),
    M("optional-patch-option-ignored-by-the-build", "kit", "    patches = apply_overlay(srcdir, kroot, archive, optional)", "    patches = apply_overlay(srcdir, kroot, archive)"),
    M("optional-patch-option-ignored-by-the-command-line", "kit", "                 with_patches=a.with_patch)", "                 with_patches=())"),
    M("optional-patch-option-ignored-by-the-overlay", "kit", "    return records + patch_records(kroot, names)", "    return records"),
    M("optional-patch-applied-twice-when-named-twice", "kit", "    return [n for n in OPTIONAL_PATCHES if n in set(requested)]", "    return sorted(requested, key=list(OPTIONAL_PATCHES).index)"),
    M("optional-patch-order-is-the-order-asked", "kit", "    return [n for n in OPTIONAL_PATCHES if n in set(requested)]", "    return list(dict.fromkeys(requested))"),
    M("optional-patch-applied-again-over-itself", "kit", "            if asked and state == \"absent\":\n                texts[rel]", "            if asked:\n                texts[rel]"),
    M("optional-patch-records-before-the-overlay", "kit", "    return records + patch_records(kroot, names)", "    return patch_records(kroot, names) + records"),
    M("optional-patch-unknown-name-ignored", "kit", "    if unknown:\n        raise KitRefused(\"unknown optional patch", "    if False:\n        raise KitRefused(\"unknown optional patch"),
    M("optional-patch-unrequested-but-present-accepted", "kit", "            if state == \"present\" and not asked:", "            if False:"),
    M("optional-patch-applied-to-text-it-does-not-fit", "kit", "            if asked and state == \"other\":", "            if False:"),
    M("optional-patch-missing-tree-file-ignored", "kit", "                if asked:\n                    raise KitRefused(\"%s lacks %s: not the QEMU source tree the patch", "                if False:\n                    raise KitRefused(\"%s lacks %s: not the QEMU source tree the patch"),
    M("optional-patch-not-in-the-manifest", "kit", '        "optional_patches": optional,\n', '        "optional_patches": [],\n'),
    M("optional-patch-record-without-hash", "kit", '"patch": n, "sha256": sha256_file(Path(kroot) / path)}', '"patch": n, "sha256": "0" * 64}'),
    M("optional-patch-build-named-as-a-plain-one", "kit", '"-dirty" if dirty else "", "".join("-with-" + n for n in patches))', '"-dirty" if dirty else "", "")'),
    M("optional-patch-version-string-plain", "kit", '("-dirty" if ident["dirty"] else "") + "".join("+" + n for n in optional)', '("-dirty" if ident["dirty"] else "")'),
    M("optional-patch-plan-version-string-plain", "kit", 'pkgversion="kayfabe-<revision>" + "".join("+" + n for n in optional))', 'pkgversion="kayfabe-<revision>")'),
    M("optional-patch-readme-silent", "kit", '+ "".join(PATCHED_README % {"patch": n} for n in optional))', ')'),
    M("optional-patch-not-in-the-source-tarball", "kit", ' "tools/qemu-patches",', ""),
    M("optional-patch-hunk-counts-not-checked", "kit", "        if want != [0, 0]:\n            raise KitRefused(\"%s: a hunk does not have", "        if False:\n            raise KitRefused(\"%s: a hunk does not have"),
    M("optional-patch-stray-text-accepted", "kit", "        elif line.strip():\n            raise KitRefused(\"%s: not a plain edit", "        elif False:\n            raise KitRefused(\"%s: not a plain edit"),
    M("optional-patch-written-before-the-device-checks", "kit", "    names = patch_names(with_patches)\n    patched = _patched_files(qemu_tree, kroot, names)\n", "    names = patch_names(with_patches)\n    patched = _patched_files(qemu_tree, kroot, names)\n    for _rel, _text in patched.items():\n        (qemu_tree / _rel).write_text(_text)\n"),
    M("optional-patch-empty-file-accepted", "kit", "    if not files or not all(files.values()):", "    if False:"),
    M("optional-patch-match-inside-a-line", "kit", "        if at == 0 or text[at - 1] == \"\\n\":\n            out.append(at)", "        out.append(at)"),
    M("set-patches-leaves-an-unrequested-patch-in", "kit", "                if state == \"present\":\n                    plan[rel] = _replace_hunks(text, hunks, reverse=True)", "                pass"),
    M("set-patches-does-not-apply", "kit", "                if state == \"absent\":\n                    plan[rel] = _replace_hunks(text, hunks)", "                pass"),
    M("set-patches-foreign-text-accepted", "kit", "            if state == \"other\":\n                raise KitRefused(\"%s: %s is not the QEMU %s text the optional patch %s changes or contains\"", "            if False:\n                raise KitRefused(\"%s: %s is not the QEMU %s text the optional patch %s changes or contains\""),
    M("set-patches-command-gone", "kit", '    w = sub.add_parser("set-patches"', '    w = sub.add_parser("set-patches-x"'),
    M("patched-bench-build-changes-the-plain-tree", "wrap", "    copy = work / tree.name\n", "    copy = tree\n"),
    M("patched-bench-build-shares-the-plain-build-dir", "wrap", '    build = work / "qemu-build-kf3"', '    build = tree.parent / "qemu-build-kf3"'),
    M("patched-bench-build-forgets-the-patch-names", "wrap", "        done = kit.set_tree_patches(copy, names)", "        done = kit.set_tree_patches(copy, [])"),
    M("patched-bench-build-copy-not-named-by-patch", "wrap", '    work = tree.parent / ("patched-" + "+".join(names))', '    work = tree.parent / "patched"'),
    M("patched-bench-build-recopies-over-the-copy", "wrap", "        if not copy.exists():", "        if True:"),
    M("patched-bench-build-accepts-any-name", "wrap", "choices=sorted(kit.OPTIONAL_PATCHES), ", ""),
    M("patched-bench-build-needs-no-name", "wrap", "required=True, ", "required=False, default=[], "),
    M("patched-bench-build-refusal-ignored", "wrap", '        print("KIT_REFUSED: %s" % e, file=sys.stderr)\n        return 1', '        print("KIT_REFUSED: %s" % e, file=sys.stderr)\n        return 0'),
    M("patched-bench-build-accepts-a-non-tree", "wrap", '    if not (tree / "VERSION").is_file():', "    if False:"),
    M("plain-bench-build-knows-a-patch", "sh", "grep -q \"subdir('kf3')\"", "python3 -I \"$REPO/scripts/release/build_qemu.py\" set-patches \"$QEMU\" --with-patch hyperv-wake\ngrep -q \"subdir('kf3')\""),
    # archives
    M("tar-mtime-from-files", "kit", "info.mtime, info.uid, info.gid, info.uname, info.gname, info.mode = epoch, 0, 0, \"\", \"\", mode",
      "info.mtime, info.uid, info.gid, info.uname, info.gname, info.mode = (int(payload.stat().st_mtime) if isinstance(payload, Path) else epoch), 0, 0, \"\", \"\", mode"),
    M("tar-unsorted", "kit", "    entries.sort(key=lambda e: e[0])\n", ""),
    M("tar-modes-from-files", "kit", 'out.append((rel, "file", 0o755 if path.stat().st_mode & 0o111 else 0o644, path))', 'out.append((rel, "file", path.stat().st_mode & 0o777, path))'),
    M("tar-links-followed", "kit", 'out.append((rel, "link", 0o777, os.readlink(path)))', 'out.append((rel, "file", 0o644, path))'),
    M("tar-owner-kept", "kit", 'info.mtime, info.uid, info.gid, info.uname, info.gname, info.mode = epoch, 0, 0, "", "", mode', 'info.mtime, info.uid, info.gid, info.uname, info.gname, info.mode = epoch, 1000, 1000, "builder", "builder", mode'),
    M("extract-absolute-link-kept-as-error", "kit", "                if m.issym() and os.path.isabs(m.linkname):\n                    dropped.add(m.name)", "                if False:\n                    dropped.add(m.name)"),
    # manifest
    M("manifest-unsorted", "kit", "json.dumps(doc, sort_keys=True,", "json.dumps(doc, sort_keys=False,"),
    M("manifest-path-check-dropped", "kit", "        if needle and needle in text:", "        if False:"),
    M("manifest-patches-dropped", "kit", '        "patches": patches,\n', '        "patches": [],\n'),
    M("manifest-tree-digest-after-overlay", "kit", '        source = {"kind": "tree", "tree_sha256": tree_digest(qi.path), "anchors": ["user"]}\n        shutil.copytree(qi.path, src, symlinks=True, ignore=shutil.ignore_patterns(".git"))',
      '        shutil.copytree(qi.path, src, symlinks=True, ignore=shutil.ignore_patterns(".git"))\n        source = {"kind": "tree", "tree_sha256": "0" * 64, "anchors": ["user"]}'),
    # trust
    M("signed-tarball-pin-not-compared", "kit", "        if not pinned:\n            raise KitRefused", "        if False:\n            raise KitRefused"),
    M("pin-size-ignored", "kit", "    return Path(tarball).stat().st_size == size and sha256_file(tarball) == sha256", "    return sha256_file(tarball) == sha256"),
    M("pin-hash-ignored", "kit", "    return Path(tarball).stat().st_size == size and sha256_file(tarball) == sha256", "    return Path(tarball).stat().st_size == size"),
    M("signed-tarball-lacks-the-pin-anchor", "kit", '        anchors = ["qemu-release-signature", "upstream-pin"]\n    elif pinned:', '        anchors = ["qemu-release-signature"]\n    elif pinned:'),
    M("source-tarball-signature-optional", "kit", "    if qi.kind == \"official\" or qi.require_signature or sig.is_file():", "    if qi.kind == \"official\" or sig.is_file():"),
    M("version-not-checked-after-extraction", "kit", "    version = read_qemu_version(tops[0])\n    check_supported(version)\n    tops[0].rename(src)", "    version = read_qemu_version(tops[0])\n    tops[0].rename(src)"),
    M("signature-fetch-skips-kf-trust", "kit", '        kf_trust.resolve("qemu-source-signature", sig, mode=kf_trust.MODE_GIT_ONLY)', "        pass"),
    M("attached-signature-accepted", "kit", '        if ":signature packet:" not in packets or re.search(r":(?:literal data|compressed|onepass_sig) packet:", packets):', "        if False:"),
    M("extraction-filter-removed", "kit", "tarfile.data_filter(member, path)", "member"),
    M("qemu-notices-not-packed", "kit", "    for member, target in QEMU_NOTICES:", "    for member, target in ():"),
    M("rust-notices-not-packed", "kit", "    for name, data in sorted(rust_notices(env, kroot).items()):", "    for name, data in ():"),
    M("readme-promises-a-source-tarball-unconditionally", "kit", "If it\nnames a source tarball (`source_tarball`), that `-source.tar.xz` is", "The `-source.tar.xz` published next to this file is"),
    M("readme-depends-on-the-source-tarball", "kit", '(pkg / "README.txt").write_text(README % {', '(pkg / "README.txt").write_text(("" if source_tarball is None else "x") + README % {'),
    M("readme-drops-the-users-own-source-case", "kit", "If its `qemu.source.anchors` is `[\"user\"]`, the QEMU\nsource was supplied by whoever built this, and the corresponding source is that QEMU source plus the kayfabe tree at the\nrevision above.\n", ""),
    M("building-text-promises-identity", "kit", "The binary tarball is expected to be byte-identical to the published one on the toolchain its manifest records; that is measured only for two builds on one machine, not across machines. Compare the manifest first.", "The binary tarball is byte-identical to the published one."),
    M("rust-notices-optional", "kit", '    if "COPYRIGHT-library.html" not in found:\n        raise KitRefused', '    if False:\n        raise KitRefused'),
    M("missing-tarball-not-checked", "kit", '        tarball, sig = require_file(qi.path, "the QEMU tarball"), qi.sig or Path(str(qi.path) + ".sig")', '        tarball, sig = qi.path, qi.sig or Path(str(qi.path) + ".sig")'),
    M("require-file-accepts-a-directory", "kit", '    if not path.is_file():\n        raise KitRefused("%s %s is not a file"', '    if False:\n        raise KitRefused("%s %s is not a file"'),
    M("require-file-accepts-a-missing-path", "kit", '    if not path.exists():\n        raise KitRefused("%s %s does not exist"', '    if False:\n        raise KitRefused("%s %s does not exist"'),
    M("source-command-skips-the-tarball-check", "kit", '    require_file(tarball, "the QEMU tarball")\n    signer = verify_signature', '    signer = verify_signature'),
    M("plan-skips-the-tarball-check", "kit", '    if qi.kind == "tarball":\n        require_file(qi.path, "the QEMU tarball")\n    version = ', '    version = '),
    M("signature-signer-unchecked", "kit", "    if primary not in allowed and signing not in allowed:", "    if False:"),
    M("signature-key-not-imported", "kit", '        if _gpg(home, "--import", str(key_file)).returncode != 0:', '        if False:'),
    M("signature-callers-keyring", "kit", '"--homedir", str(homedir)', '"--homedir", os.environ.get("GNUPGHOME", str(homedir))'),
    M("pin-file-extra-lines-ignored", "kit", '        raise KitRefused("%s: not a signer or signature line: %r" % (path, line))', "        continue"),
    # identity and environment
    M("dirty-checkout-accepted", "kit", '        dirty = bool(_git(root, "status", "--porcelain", "--untracked-files=no", "--", *SOURCE_PATHS).strip())', "        dirty = False"),
    M("stamp-epoch-not-checked-as-int", "kit", 'type(info.get("source_date_epoch")) is int and info["source_date_epoch"] > 0)', "True)"),
    M("environment-inherited", "kit", "    env = {k: caller[k] for k in PASS_ENV if k in caller}", "    env = dict(caller)"),
    M("epoch-from-caller", "kit", '"SOURCE_DATE_EPOCH": str(epoch)', '"SOURCE_DATE_EPOCH": caller.get("SOURCE_DATE_EPOCH", str(epoch))'),
    M("rustflags-not-set", "kit", '"RUSTFLAGS": " ".join("--remap-path-prefix=%s=%s" % pair for pair in remaps),', '"RUSTFLAGS": "",'),
    M("cargo-home-not-remapped", "kit", "(str(cargo_home), MAP_CARGO), ", ""),
    M("timezone-not-fixed", "kit", '"TZ": "UTC", ', ""),
    # orchestration
    M("cargo-not-locked", "kit", '"--release", "--locked"', '"--release"'),
    M("install-without-destdir", "kit", '    install_env = {**env, "DESTDIR": str(stage)}', "    install_env = env"),
    M("failure-ignored", "kit", "    if rc != 0:\n        tail = \"\"", "    if False:\n        tail = \"\""),
    M("foreign-workdir-wiped", "kit", "        if not (workdir / WORK_MARK).is_file():", "        if False:"),
    M("existing-assets-overwritten", "kit", "    if out.exists() and any((out / n).exists() for n in names.values()):", "    if False:"),
    M("licence-files-not-packed", "kit", '        if (kroot / doc).is_file():\n            shutil.copyfile(kroot / doc, pkg / doc)', "        pass"),
    # comparing builds
    M("compare-reports-nothing", "kit", '        diff.append("%s differs (sha256 %s vs %s)" % (name, sha256_bytes(x)[:16], sha256_bytes(y)[:16]))', "        pass"),
    M("compare-ignores-keys-always", "kit", "            for key in ignore_keys:", '            for key in ("source_tarball", "a"):'),
    M("compare-misses-one-sided-files", "kit", '    diff = ["only in %s: %s" % (side, n) for side, mine, other in (("a", names_a, names_b), ("b", names_b, names_a)) for n in mine if n not in other]', "    diff = []"),
    # command line
    M("two-qemu-inputs-allowed", "kit", "    if len(given) > 1:", "    if len(given) > 9:"),
    M("refusal-exit-code", "kit", '        print("KIT_REFUSED: %s" % e, file=sys.stderr)\n        return 1', '        print("KIT_REFUSED: %s" % e, file=sys.stderr)\n        return 0'),
    # workflow
    M("wf-push-trigger", "wf", "on:\n  workflow_dispatch:", "on:\n  push:\n    branches: [master]\n  workflow_dispatch:"),
    M("wf-permissions-widened", "wf", "permissions: {}\n", "permissions: write-all\n"),
    M("wf-build-can-write-contents", "wf", "    permissions:\n      contents: read\n    outputs:", "    permissions:\n      contents: write\n    outputs:"),
    M("wf-build-holds-oidc", "wf", "    permissions:\n      contents: read\n    outputs:", "    permissions:\n      contents: read\n      id-token: write\n    outputs:"),
    M("wf-release-published", "wf", " --draft --target", " --target"),
    M("wf-attest-off-master", "wf", "    if: ${{ github.ref == 'refs/heads/master' && needs.gate.result == 'success' && needs.build.result == 'success' }}\n", "    if: ${{ needs.gate.result == 'success' && needs.build.result == 'success' }}\n"),
    M("wf-attest-without-the-gate", "wf", "needs.gate.result == 'success' && needs.build.result == 'success' }}", "needs.build.result == 'success' }}"),
    M("wf-attest-without-the-build-result", "wf", "needs.gate.result == 'success' && needs.build.result == 'success' }}", "needs.gate.result == 'success' }}"),
    M("wf-unpinned-action", "wf", "      - uses: actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333 # v8.0.2\n        with:\n          name: qemu-release-assets\n          path: assets\n      - name: Every file", "      - uses: actions/download-artifact@v8\n        with:\n          name: qemu-release-assets\n          path: assets\n      - name: Every file"),
    M("wf-secret-read", "wf", "          GH_TOKEN: ${{ github.token }}\n          TAG:", "          GH_TOKEN: ${{ secrets.RELEASE_TOKEN }}\n          TAG:"),
    M("wf-reproducibility-check-removed", "wf", "build_qemu.py reproduce --source-tarball", "build_qemu.py build --source-tarball"),
    M("wf-draft-off-master", "wf", "    needs: [gate, build, attest]\n    if: ${{ github.ref == 'refs/heads/master' && needs.gate.result == 'success' && needs.attest.result == 'success' }}\n", "    needs: [gate, build, attest]\n"),
    M("wf-gate-not-awaited", "wf", "    needs: gate\n    if: ${{ !cancelled() && (github.ref != 'refs/heads/master' || needs.gate.result == 'success') }}\n", ""),
    M("wf-build-runs-after-a-cancelled-gate", "wf", "|| needs.gate.result == 'success') }}", "|| needs.gate.result != 'failure') }}"),
    M("wf-tag-unchecked-at-release", "wf", '          [[ "$TAG" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || { echo "bad tag name"; exit 1; }\n          # only a 404', "          # only a 404"),
    M("wf-tag-in-shell-text", "wf", "gh release create \"$TAG\"", "gh release create \"${{ inputs.tag }}\""),
    M("wf-checkout-keeps-credentials", "wf", "          persist-credentials: false\n      - name: Check the tag name", "          persist-credentials: true\n      - name: Check the tag name"),
    M("wf-verify-loop-signer-workflow-dropped", "wf", ' --signer-workflow "$GITHUB_REPOSITORY/.github/workflows/release-qemu.yml" --source-ref', " --source-ref"),
    M("wf-verify-loop-source-ref-dropped", "wf", " --source-ref refs/heads/master --deny-self-hosted-runners\n            upload+=", " --deny-self-hosted-runners\n            upload+="),
    M("wf-verify-loop-hosted-runner-pin-dropped", "wf", "refs/heads/master --deny-self-hosted-runners\n            upload+=", "refs/heads/master\n            upload+="),
    M("wf-verify-loop-removed", "wf", '            gh attestation verify "$f" --repo', '            true "$f" --repo'),
    M("wf-notes-signer-workflow-dropped", "wf", " --signer-workflow ${GITHUB_REPOSITORY}/.github/workflows/release-qemu.yml --source-ref", " --source-ref"),
    M("wf-notes-source-ref-dropped", "wf", " --source-ref refs/heads/master --deny-self-hosted-runners\" \\", " --deny-self-hosted-runners\" \\"),
    M("wf-notes-hosted-runner-pin-dropped", "wf", "refs/heads/master --deny-self-hosted-runners\" \\", "refs/heads/master\" \\"),
    M("doc-verify-signer-workflow-dropped", "doc", " --signer-workflow reindertpelsma/kayfabe/.github/workflows/release-qemu.yml --source-ref", " --source-ref"),
    M("doc-verify-source-ref-dropped", "doc", " --source-ref refs/heads/master --deny-self-hosted-runners\nsha256sum", " --deny-self-hosted-runners\nsha256sum"),
    M("doc-verify-hosted-runner-pin-dropped", "doc", " --deny-self-hosted-runners\nsha256sum -c", "\nsha256sum -c"),
    M("wf-attests-binary-only", "wf", "            assets/*-source.tar.xz\n            assets/*-manifest.json\n", ""),
    M("wf-attest-checksum-digest-unchecked", "wf", '= "$SUMS" ] ||', '!= "" ] ||'),
    M("wf-attest-strict-check-dropped", "wf", "sha256sum --check --strict SHA256SUMS", "sha256sum SHA256SUMS"),
    M("wf-attest-extra-files-allowed", "wf", '[ "$listed" = "$present" ] ||', 'true ||'),
    M("wf-build-digest-output-dropped", "wf", "      sums: ${{ steps.sums.outputs.sums }}\n", "      other: ${{ steps.sums.outputs.sums }}\n"),
    M("wf-attest-digest-from-elsewhere", "wf", "SUMS: ${{ needs.build.outputs.sums }}", "SUMS: ${{ github.sha }}"),
    M("wf-draft-existing-tag-allowed", "wf", '          if lookup="$(gh api "repos/$GITHUB_REPOSITORY/git/ref/tags/$TAG" 2>&1)"; then echo "the tag $TAG already exists"; exit 1; fi\n', ""),
    M("wf-tag-lookup-fails-open", "wf", '          case "$lookup" in *"HTTP 404"*) ;; *) echo "cannot check whether the tag $TAG exists: $lookup"; exit 1;; esac\n', ""),
    M("wf-tag-lookup-takes-any-error-as-absent", "wf", 'case "$lookup" in *"HTTP 404"*) ;; *) echo', 'case "$lookup" in *) ;; *) echo'),
    M("wf-tag-lookup-old-fail-open-form", "wf", '          if lookup="$(gh api "repos/$GITHUB_REPOSITORY/git/ref/tags/$TAG" 2>&1)"; then echo "the tag $TAG already exists"; exit 1; fi\n          case "$lookup" in *"HTTP 404"*) ;; *) echo "cannot check whether the tag $TAG exists: $lookup"; exit 1;; esac\n', '          if gh api "repos/$GITHUB_REPOSITORY/git/ref/tags/$TAG" > /dev/null 2>&1; then echo "the tag $TAG already exists"; exit 1; fi\n'),
    M("wf-404-matched-anywhere-in-the-answer", "wf", '*"HTTP 404"*) ;;', '*"404"*) ;;'),
    M("wf-draft-existing-release-allowed", "wf", '          if printf \'%s\\n\' "$releases" | grep -Fxq -- "$TAG"; then echo "a release for $TAG already exists"; exit 1; fi\n', ""),
    M("wf-release-listing-failure-ignored", "wf", '|| { echo "cannot list the releases"; exit 1; }', "|| true"),
    M("wf-per-pattern-count-check-removed", "wf", '            [ "${#matches[@]}" -eq 1 ] || { echo "expected exactly one file for $pat, found ${#matches[@]}"; exit 1; }\n', ""),
    M("wf-per-pattern-count-allows-two", "wf", '[ "${#matches[@]}" -eq 1 ] ||', '[ "${#matches[@]}" -le 2 ] ||'),
    M("wf-glob-not-expanded-to-an-array", "wf", "            matches=($pat)\n", '            matches=("$pat")\n'),
    M("wf-draft-uploads-everything", "wf", '-- "${upload[@]}"', "-- *"),
    M("wf-draft-extra-file-allowed", "wf", '          [ "$(ls -A | wc -l)" -eq 4 ] || { echo "the artifact holds files that were not verified"; exit 1; }\n', ""),
    M("wf-draft-cannot-read-attestations", "wf", "      contents: write\n      attestations: read\n", "      contents: write\n"),
    M("wf-reproduce-inside-the-checkout", "wf", '--out "$RUNNER_TEMP/repro"', "--out dist/repro"),
]


def sandbox(parent, name, key, text):
    root = Path(tempfile.mkdtemp(prefix=name + "-", dir=parent))
    (root / "scripts/release").mkdir(parents=True)
    (root / "scripts/ci").mkdir(parents=True)
    (root / ".github/workflows").mkdir(parents=True)
    for rel in REAL_COPIES:
        shutil.copyfile(ROOT / rel, root / rel)
    for rel in ("scripts/release/build_qemu.py", ".github/workflows/release-qemu.yml"):
        shutil.copyfile(ROOT / rel, root / rel)
    (root / "docs/howto").mkdir(parents=True)
    for child in (ROOT / "docs").iterdir():
        if child.name != "howto":
            os.symlink(child, root / "docs" / child.name)
    for child in (ROOT / "docs/howto").iterdir():
        os.symlink(child, root / "docs/howto" / child.name)
    (root / "docs/howto/building.md").unlink()
    shutil.copyfile(ROOT / "docs/howto/building.md", root / "docs/howto/building.md")
    for other in (ROOT / ".github/workflows").glob("*.yml"):
        if not (root / ".github/workflows" / other.name).exists():
            shutil.copyfile(other, root / ".github/workflows" / other.name)
    # everything else the tests look at is read only: link it
    for rel in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "crates", "firmware", "cuda", "qemu", "tools", "LICENSE", "LICENSE-APACHE",
                "LICENSE-GPL", "THIRD_PARTY_NOTICES.md", "README.md", "traces"):
        os.symlink(ROOT / rel, root / rel)
    for rel in ("scripts/ci/kf_trust.py", "scripts/ci/kf_fetch.py", "scripts/ci/qemu_source.py", "scripts/ci/trust_manifest.json"):
        os.symlink(ROOT / rel, root / rel)
    (root / "scripts/bench").mkdir()
    for child in (ROOT / "scripts/bench").iterdir():
        if child.name not in ("build_kf3.sh", "build_kf3_patched.py"):
            os.symlink(child, root / "scripts/bench" / child.name)
    for own in ("build_kf3.sh", "build_kf3_patched.py"):
        shutil.copyfile(ROOT / "scripts/bench" / own, root / "scripts/bench" / own)
    (root / FILES[key]).write_text(text)
    return root


def run(root):
    env = {"PATH": os.environ.get("PATH", ""), "HOME": os.environ.get("HOME", ""), "PYTHONDONTWRITEBYTECODE": "1"}
    p = subprocess.run([sys.executable, "-c", COLLECT, SUITE], cwd=root / "scripts/ci", env={**env, "PYTHONPATH": str(root / "scripts/ci")},
                       capture_output=True, text=True, timeout=600)
    for line in p.stdout.splitlines():
        if line.startswith("KF_MUTATION_RESULT="):
            return json.loads(line.split("=", 1)[1])
    raise RuntimeError("no result from the mutated suite: " + (p.stderr or p.stdout)[-400:])


def judge(parent, mutant):
    name, key, old, new = mutant
    if SOURCES[key].count(old) != 1:
        return name, None, "pattern found %d times" % SOURCES[key].count(old)
    root = sandbox(parent, name, key, SOURCES[key].replace(old, new))
    try:
        res = run(root)
    finally:
        shutil.rmtree(root, ignore_errors=True)
    return name, res["killers"], None


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("only", nargs="?")
    a = ap.parse_args(argv)
    chosen = [m for m in MUTANTS if not a.only or a.only in m[0]]
    names = [m[0] for m in MUTANTS]
    if len(set(names)) != len(names):
        print("duplicate mutant names")
        return 2
    bad = 0
    with tempfile.TemporaryDirectory(prefix="relq-mut-") as parent:
        base = sandbox(parent, "baseline", "kit", SOURCES["kit"])
        try:
            res = run(base)
        finally:
            shutil.rmtree(base, ignore_errors=True)
        if res["killers"]:
            print("BASELINE RED:", res["killers"])
            return 1
        print("baseline green: %d tests" % res["tests"])
        with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
            for name, killers, problem in pool.map(lambda m: judge(parent, m), chosen):
                if problem:
                    print("BAD MUTANT %s: %s" % (name, problem))
                    bad += 1
                elif len(killers) < 2:
                    print("WEAK %s: killers=%s" % (name, killers))
                    bad += 1
                else:
                    print("killed %s by %d tests (%s ...)" % (name, len(killers), ", ".join(killers[:2])))
    print("%d mutants, %d problems" % (len(chosen), bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
