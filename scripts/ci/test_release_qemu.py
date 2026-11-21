"""Tests of the QEMU release kit (scripts/release/build_qemu.py) and of the workflow that publishes its output.

STATUS: LIVE, 2026-11-21. GPU-free, network-free, and no compiler runs: the build steps are driven through a fake runner. The
real build is exercised by `build_qemu.py reproduce` (docs/howto/building.md) and by .github/workflows/release-qemu.yml.

FALSIFIER (written before the code): the kit builds a binary that differs between two runs of the same inputs; accepts a QEMU
tree or tarball it did not verify and does not know; overlays the device twice or over a foreign file; leaks the caller's
environment or a build path into the result; writes a manifest that is not a function of its inputs; or the workflow publishes
(not a draft), runs on a push, holds a secret, runs an unpinned action, or attests only on a ref the Worker would not accept.
"""

import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

import yaml

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "scripts" / "release"))
sys.path.insert(0, str(HERE))
import build_qemu as kit  # noqa: E402

WORKFLOW = ROOT / ".github/workflows/release-qemu.yml"
HAVE_GPG = shutil.which("gpg") is not None
HAVE_GIT = shutil.which("git") is not None


def sha(data):
    return hashlib.sha256(data).hexdigest()


def write(path, text, mode=None):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(text if isinstance(text, bytes) else text.encode())
    if mode is not None:
        path.chmod(mode)
    return path


def qemu_tree(root, version="10.2.4"):
    root = Path(root)
    write(root / "VERSION", version + "\n")
    write(root / "hw/misc/meson.build", "system_ss.add(files('edu.c'))\n")
    write(root / "hw/misc/Kconfig", "config EDU\n    bool\n")
    write(root / "configure", "#!/bin/sh\n", 0o755)
    return root


def kayfabe_tree(root):
    root = Path(root)
    write(root / "qemu/hw/misc/kf3/kf3.c", "int kf3;\n")
    write(root / "qemu/hw/misc/kf3/kf3.h", "#define KF3_ABI 36\n")
    write(root / "qemu/hw/misc/kf3/kf3_gop.h", "/* gop */\n")
    write(root / "qemu/hw/misc/kf3/meson.build", "kf3_archive = 1\n")
    write(root / "qemu/hw/misc/kf3/tests/test_x.py", "x = 1\n")
    return root


def archive_file(path, data=b"!<arch>\nstub"):
    return write(path, data)


def git(cwd, *args, env=None):
    e = {**os.environ, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@e", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@e",
         "GIT_AUTHOR_DATE": "2026-01-02T03:04:05Z", "GIT_COMMITTER_DATE": "2026-01-02T03:04:05Z", "GIT_CONFIG_GLOBAL": "/dev/null",
         "GIT_CONFIG_SYSTEM": "/dev/null"}
    return subprocess.run(["git", *args], cwd=cwd, env=e, check=True, capture_output=True, text=True).stdout.strip()


class Tmp(unittest.TestCase):
    def setUp(self):
        self._d = tempfile.TemporaryDirectory()
        self.addCleanup(self._d.cleanup)
        self.tmp = Path(self._d.name)


class ConfigureLine(unittest.TestCase):
    def args(self, **kw):
        base = dict(src="/w/src", build="/w/build", kroot="/w/k", pkgversion="kayfabe-abcdef12")
        base.update(kw)
        return kit.configure_args(**base)

    def test_one_target_and_no_gui_front_end(self):
        a = self.args()
        self.assertIn("--target-list=x86_64-softmmu", a)
        self.assertEqual([x for x in a if x.startswith("--target-list")], ["--target-list=x86_64-softmmu"])
        for gui in ("--enable-gtk", "--enable-sdl", "--enable-curses", "--enable-opengl"):
            self.assertNotIn(gui, a)
        for off in ("--disable-gtk", "--disable-sdl"):
            self.assertIn(off, a)

    def test_the_tested_feature_set_is_kept(self):
        a = self.args()
        for flag in ("--enable-kvm", "--enable-system", "--enable-pixman", "--enable-vnc", "--enable-tpm", "--enable-slirp",
                     "--without-default-features", "--disable-docs", "--disable-tools", "--disable-werror"):
            self.assertIn(flag, a)

    def test_no_network_during_configure(self):
        self.assertIn("--disable-download", self.args())

    def test_install_is_stripped_and_carries_no_debug_info(self):
        a = self.args()
        self.assertIn("--enable-strip", a)
        self.assertIn("--disable-debug-info", a)

    def test_every_build_path_is_mapped_to_a_fixed_name(self):
        flags = [x for x in self.args() if x.startswith("--extra-cflags=")]
        self.assertEqual(len(flags), 1)
        text = flags[0]
        for real, fixed in (("/w/src", "/qemu-src"), ("/w/build", "/qemu-build"), ("/w/k", "/kayfabe-src")):
            self.assertIn("-ffile-prefix-map=%s=%s" % (real, fixed), text)

    def test_pkgversion_names_the_kayfabe_revision(self):
        self.assertIn("--with-pkgversion=kayfabe-abcdef12", self.args())

    def test_recorded_line_does_not_depend_on_where_the_build_ran(self):
        one = kit.configure_line(self.args(src="/w1/src", build="/w1/build", kroot="/w1/k"), "/w1/src", "/w1/build", "/w1/k")
        two = kit.configure_line(self.args(src="/other/q", build="/other/b", kroot="/other/k"), "/other/q", "/other/b", "/other/k")
        self.assertEqual(one, two)
        self.assertNotIn("/w1", one)
        self.assertTrue(one.startswith("<qemu-src>/configure "))


class VersionCheck(Tmp):
    def test_known_version_is_accepted(self):
        self.assertEqual(kit.read_qemu_version(qemu_tree(self.tmp / "q")), "10.2.4")
        kit.check_supported("10.2.4")

    def test_unknown_version_is_refused_and_the_known_list_is_named(self):
        with self.assertRaises(kit.KitRefused) as cm:
            kit.check_supported("9.0.0")
        text = str(cm.exception)
        self.assertIn("9.0.0", text)
        for known in kit.SUPPORTED_QEMU:
            self.assertIn(known, text)

    def test_a_prefix_of_a_known_version_is_not_known(self):
        for bad in ("10.2", "10.2.4-rc1", "10.2.40", ""):
            with self.assertRaises(kit.KitRefused):
                kit.check_supported(bad)

    def test_tree_without_version_file_is_refused(self):
        with self.assertRaises(kit.KitRefused):
            kit.read_qemu_version(self.tmp)

    def test_tree_version_with_junk_is_refused(self):
        write(self.tmp / "VERSION", "10.2.4\nextra\n")
        with self.assertRaises(kit.KitRefused):
            kit.read_qemu_version(self.tmp)


class Overlay(Tmp):
    def setUp(self):
        super().setUp()
        self.k = kayfabe_tree(self.tmp / "k")
        self.q = qemu_tree(self.tmp / "q")
        self.a = archive_file(self.tmp / "libkf_qemu.a")

    def apply(self):
        return kit.apply_overlay(self.q, self.k, self.a)

    def test_device_sources_and_archive_land_in_hw_misc_kf3(self):
        self.apply()
        d = self.q / "hw/misc/kf3"
        self.assertEqual(sorted(p.name for p in d.iterdir()), ["kf3.c", "kf3.h", "kf3_gop.h", "libkf_qemu.a", "meson.build"])
        self.assertEqual((d / "libkf_qemu.a").read_bytes(), self.a.read_bytes())

    def test_the_tests_directory_is_not_copied(self):
        self.apply()
        self.assertFalse((self.q / "hw/misc/kf3/tests").exists())

    def test_the_two_hunks_are_appended_once(self):
        self.apply()
        meson = (self.q / "hw/misc/meson.build").read_text()
        kconfig = (self.q / "hw/misc/Kconfig").read_text()
        self.assertEqual(meson.count("subdir('kf3')"), 1)
        self.assertEqual(kconfig.count("config KF3"), 1)
        self.assertTrue(meson.startswith("system_ss.add(files('edu.c'))\n"))
        self.assertIn("depends on PCI", kconfig)
        self.assertIn("default y if TEST_DEVICES", kconfig)

    def test_applying_twice_changes_nothing_more(self):
        first = self.apply()
        digest = kit.tree_digest(self.q)
        second = self.apply()
        self.assertEqual(kit.tree_digest(self.q), digest)
        self.assertEqual(first, second)

    def test_patch_records_name_every_change_with_a_hash(self):
        records = self.apply()
        by_path = {r["path"]: r for r in records}
        self.assertEqual(by_path["hw/misc/kf3/kf3.c"]["sha256"], sha(b"int kf3;\n"))
        self.assertEqual(by_path["hw/misc/kf3/libkf_qemu.a"]["sha256"], sha(self.a.read_bytes()))
        self.assertEqual(by_path["hw/misc/meson.build"]["kind"], "append-line")
        self.assertEqual(by_path["hw/misc/Kconfig"]["kind"], "append-stanza")
        self.assertEqual([r["path"] for r in records], sorted(r["path"] for r in records if r["kind"] == "add-file")
                         + ["hw/misc/Kconfig", "hw/misc/meson.build"])

    def test_a_foreign_file_in_the_device_directory_is_refused(self):
        write(self.q / "hw/misc/kf3/notes.txt", "mine")
        with self.assertRaises(kit.KitRefused):
            self.apply()
        self.assertFalse((self.q / "hw/misc/kf3/kf3.c").exists())

    def test_a_tree_without_the_hw_misc_files_is_refused(self):
        (self.q / "hw/misc/Kconfig").unlink()
        with self.assertRaises(kit.KitRefused):
            self.apply()

    def test_a_missing_archive_is_refused(self):
        self.a.unlink()
        with self.assertRaises(kit.KitRefused):
            self.apply()

    def test_a_missing_device_source_is_refused(self):
        (self.k / "qemu/hw/misc/kf3/kf3.c").unlink()
        with self.assertRaises(kit.KitRefused):
            self.apply()

    def test_a_tree_that_already_has_the_hunks_keeps_exactly_one_each(self):
        write(self.q / "hw/misc/meson.build", "x\nsubdir('kf3')\n")
        write(self.q / "hw/misc/Kconfig", "config KF3\n    bool\n")
        self.apply()
        self.assertEqual((self.q / "hw/misc/meson.build").read_text().count("subdir('kf3')"), 1)
        self.assertEqual((self.q / "hw/misc/Kconfig").read_text().count("config KF3"), 1)

    def test_abi_number_is_read_from_the_header(self):
        self.assertEqual(kit.kf3_abi(self.k), 36)
        write(self.k / "qemu/hw/misc/kf3/kf3.h", "no abi here\n")
        with self.assertRaises(kit.KitRefused):
            kit.kf3_abi(self.k)


class TreeDigest(Tmp):
    def make(self, name="t"):
        root = self.tmp / name
        write(root / "a.txt", "alpha")
        write(root / "d/b.txt", "beta")
        write(root / "run.sh", "#!/bin/sh\n", 0o755)
        return root

    def test_same_content_same_digest_whatever_the_mtime_or_location(self):
        a, b = self.make("a"), self.make("b")
        os.utime(b / "a.txt", (1, 1))
        self.assertEqual(kit.tree_digest(a), kit.tree_digest(b))

    def test_content_path_and_exec_bit_each_change_the_digest(self):
        base = kit.tree_digest(self.make("base"))
        c = self.make("c")
        write(c / "a.txt", "alphA")
        self.assertNotEqual(kit.tree_digest(c), base)
        p = self.make("p")
        (p / "a.txt").rename(p / "z.txt")
        self.assertNotEqual(kit.tree_digest(p), base)
        x = self.make("x")
        (x / "a.txt").chmod(0o755)
        self.assertNotEqual(kit.tree_digest(x), base)

    def test_symlink_target_matters_and_is_not_followed(self):
        a = self.make("a")
        os.symlink("a.txt", a / "ln")
        b = self.make("b")
        os.symlink("d/b.txt", b / "ln")
        self.assertNotEqual(kit.tree_digest(a), kit.tree_digest(b))

    def test_the_git_directory_is_not_part_of_the_tree(self):
        a, b = self.make("a"), self.make("b")
        write(b / ".git/HEAD", "ref: x")
        self.assertEqual(kit.tree_digest(a), kit.tree_digest(b))


class DeterministicTar(Tmp):
    def tree(self, name="t"):
        root = self.tmp / name
        write(root / "bin/prog", "binary", 0o700)
        write(root / "share/zeta.txt", "z", 0o600)
        write(root / "share/alpha.txt", "a", 0o666)
        os.symlink("alpha.txt", root / "share/link")
        return root

    def test_same_tree_same_bytes_even_when_mtimes_and_modes_differ(self):
        a, b = self.tree("a"), self.tree("b")
        os.utime(b / "share/alpha.txt", (12345, 12345))
        (b / "share/zeta.txt").chmod(0o640)
        kit.pack_tree(a, self.tmp / "a.tar.xz", 1700000000, "pkg")
        kit.pack_tree(b, self.tmp / "b.tar.xz", 1700000000, "pkg")
        self.assertEqual((self.tmp / "a.tar.xz").read_bytes(), (self.tmp / "b.tar.xz").read_bytes())

    def test_the_epoch_is_the_only_clock(self):
        a = self.tree("a")
        kit.pack_tree(a, self.tmp / "one.tar.xz", 1700000000, "pkg")
        kit.pack_tree(a, self.tmp / "two.tar.xz", 1700000001, "pkg")
        self.assertNotEqual((self.tmp / "one.tar.xz").read_bytes(), (self.tmp / "two.tar.xz").read_bytes())
        with tarfile.open(self.tmp / "one.tar.xz") as tf:
            self.assertEqual({m.mtime for m in tf.getmembers()}, {1700000000})

    def test_members_are_sorted_owned_by_nobody_and_modes_are_normalised(self):
        kit.pack_tree(self.tree(), self.tmp / "x.tar.xz", 1700000000, "pkg")
        with tarfile.open(self.tmp / "x.tar.xz") as tf:
            members = tf.getmembers()
        names = [m.name for m in members]
        self.assertEqual(names, sorted(names))
        self.assertEqual(names[0], "pkg")
        for m in members:
            self.assertEqual((m.uid, m.gid, m.uname, m.gname), (0, 0, "", ""))
        modes = {m.name: m.mode for m in members}
        self.assertEqual(modes["pkg/bin/prog"], 0o755)
        self.assertEqual(modes["pkg/share/zeta.txt"], 0o644)
        self.assertEqual(modes["pkg/share"], 0o755)
        link = next(m for m in members if m.name == "pkg/share/link")
        self.assertTrue(link.issym())
        self.assertEqual(link.linkname, "alpha.txt")

    def test_content_survives(self):
        kit.pack_tree(self.tree(), self.tmp / "x.tar.xz", 1700000000, "pkg")
        with tarfile.open(self.tmp / "x.tar.xz") as tf:
            self.assertEqual(tf.extractfile("pkg/bin/prog").read(), b"binary")

    def test_entry_list_form_is_sorted_the_same_way(self):
        files = {"b.txt": b"b", "a.txt": b"a", "sub/c.txt": b"c"}
        kit.pack_entries(sorted(files.items(), reverse=True), self.tmp / "e.tar.xz", 5, "root")
        with tarfile.open(self.tmp / "e.tar.xz") as tf:
            names = [m.name for m in tf.getmembers()]
        self.assertEqual(names, ["root", "root/a.txt", "root/b.txt", "root/sub", "root/sub/c.txt"])


class Manifest(unittest.TestCase):
    def doc(self, **kw):
        base = {"schema": kit.SCHEMA, "qemu": {"version": "10.2.4"}, "outputs": [{"name": "x", "sha256": "0" * 64}]}
        base.update(kw)
        return base

    def test_rendering_is_sorted_stable_and_newline_terminated(self):
        a = kit.render_manifest(self.doc(z=1, a=2))
        b = kit.render_manifest(dict(reversed(list(self.doc(z=1, a=2).items()))))
        self.assertEqual(a, b)
        self.assertTrue(a.endswith("}\n"))
        self.assertLess(a.index('"a"'), a.index('"z"'))

    def test_an_absolute_path_in_a_manifest_is_refused(self):
        with self.assertRaises(kit.KitRefused):
            kit.render_manifest(self.doc(where="/work/xyz"), forbid=["/work/xyz"])

    def test_non_json_values_are_refused(self):
        with self.assertRaises(TypeError):
            kit.render_manifest(self.doc(when=object()))


class Identity(Tmp):
    @unittest.skipUnless(HAVE_GIT, "git")
    def test_revision_and_epoch_come_from_the_commit(self):
        k = kayfabe_tree(self.tmp / "k")
        git(k, "init", "-q")
        git(k, "add", "-A")
        git(k, "commit", "-q", "-m", "x")
        ident = kit.kayfabe_identity(k)
        self.assertEqual(ident["revision"], git(k, "rev-parse", "HEAD"))
        self.assertEqual(ident["source_date_epoch"], 1767323045)
        self.assertFalse(ident["dirty"])

    @unittest.skipUnless(HAVE_GIT, "git")
    def test_a_modified_tracked_file_is_dirty_and_refused_by_default(self):
        k = kayfabe_tree(self.tmp / "k")
        git(k, "init", "-q")
        git(k, "add", "-A")
        git(k, "commit", "-q", "-m", "x")
        write(k / "qemu/hw/misc/kf3/kf3.c", "changed\n")
        self.assertTrue(kit.kayfabe_identity(k, allow_dirty=True)["dirty"])
        with self.assertRaises(kit.KitRefused):
            kit.kayfabe_identity(k)

    @unittest.skipUnless(HAVE_GIT, "git")
    def test_a_modified_file_outside_the_shipped_paths_is_not_dirt(self):
        k = kayfabe_tree(self.tmp / "k")
        write(k / "docs/x.md", "x")
        git(k, "init", "-q")
        git(k, "add", "-A")
        git(k, "commit", "-q", "-m", "x")
        write(k / "docs/x.md", "changed")
        self.assertFalse(kit.kayfabe_identity(k)["dirty"])

    def test_a_source_tarball_tree_states_its_own_identity(self):
        k = kayfabe_tree(self.tmp / "k")
        write(k / "SOURCE_INFO.json", json.dumps({"revision": "a" * 40, "source_date_epoch": 99}))
        ident = kit.kayfabe_identity(k)
        self.assertEqual((ident["revision"], ident["source_date_epoch"], ident["dirty"]), ("a" * 40, 99, False))

    def test_a_tree_with_neither_git_nor_stamp_is_refused(self):
        with self.assertRaises(kit.KitRefused):
            kit.kayfabe_identity(kayfabe_tree(self.tmp / "k"))

    def test_a_malformed_stamp_is_refused(self):
        k = kayfabe_tree(self.tmp / "k")
        for bad in ('{"revision": "abc", "source_date_epoch": 1}', '{"revision": "%s", "source_date_epoch": "1"}' % ("a" * 40), "nope"):
            write(k / "SOURCE_INFO.json", bad)
            with self.assertRaises(kit.KitRefused):
                kit.kayfabe_identity(k)


class BuildEnvironment(unittest.TestCase):
    def env(self, caller=None, **kw):
        base = dict(epoch=1700000000, kroot="/w/k", cargo_home="/h/.cargo", rustup_home="/h/.rustup", target_dir="/w/t", caller=caller or {})
        base.update(kw)
        return kit.build_env(**base)

    def test_fixed_variables_are_set(self):
        e = self.env()
        self.assertEqual(e["SOURCE_DATE_EPOCH"], "1700000000")
        self.assertEqual((e["TZ"], e["LC_ALL"], e["LANG"]), ("UTC", "C", "C"))
        self.assertEqual(e["CARGO_INCREMENTAL"], "0")
        self.assertEqual(e["CARGO_TARGET_DIR"], "/w/t")

    def test_the_callers_compile_flags_and_clock_do_not_leak(self):
        caller = {"CFLAGS": "-O0", "CXXFLAGS": "-g", "LDFLAGS": "-s", "RUSTFLAGS": "-Ctarget-cpu=native", "CARGO_BUILD_RUSTFLAGS": "x",
                  "SOURCE_DATE_EPOCH": "5", "LD_PRELOAD": "/x.so", "CC": "clang", "PATH": "/usr/bin", "HOME": "/h", "TERM": "xterm"}
        e = self.env(caller)
        for name in ("CFLAGS", "CXXFLAGS", "LDFLAGS", "CARGO_BUILD_RUSTFLAGS", "LD_PRELOAD", "CC"):
            self.assertNotIn(name, e)
        self.assertEqual(e["SOURCE_DATE_EPOCH"], "1700000000")
        self.assertNotIn("native", e["RUSTFLAGS"])
        self.assertEqual((e["PATH"], e["HOME"]), ("/usr/bin", "/h"))

    def test_rust_paths_are_remapped_to_fixed_names(self):
        flags = self.env()["RUSTFLAGS"].split()
        self.assertIn("--remap-path-prefix=/w/k=/kayfabe-src", flags)
        self.assertIn("--remap-path-prefix=/h/.cargo=/cargo-home", flags)
        self.assertIn("--remap-path-prefix=/h/.rustup=/rustup-home", flags)

    def test_toolchain_selection_variables_pass_through(self):
        e = self.env({"RUSTUP_TOOLCHAIN": "1.99.0", "PKG_CONFIG_PATH": "/p", "CARGO_HOME": "/h/.cargo", "RUSTUP_HOME": "/h/.rustup"})
        self.assertEqual(e["RUSTUP_TOOLCHAIN"], "1.99.0")
        self.assertEqual(e["PKG_CONFIG_PATH"], "/p")


@unittest.skipUnless(HAVE_GPG, "gpg")
class Signature(Tmp):
    def setUp(self):
        super().setUp()
        self.home = self.tmp / "gnupg"
        self.home.mkdir(mode=0o700)
        self.genv = {**os.environ, "GNUPGHOME": str(self.home)}
        self.keys = {}
        for name in ("official", "other"):
            subprocess.run(["gpg", "--batch", "--passphrase", "", "--quick-gen-key", "%s <%s@example.invalid>" % (name, name), "rsa2048", "sign", "never"],
                           env=self.genv, check=True, capture_output=True)
            out = subprocess.run(["gpg", "--batch", "--with-colons", "--fingerprint", "%s@example.invalid" % name], env=self.genv, check=True,
                                 capture_output=True, text=True).stdout
            fpr = re.search(r"^fpr:+([0-9A-F]{40}):", out, re.M).group(1)
            asc = subprocess.run(["gpg", "--batch", "--armor", "--export", fpr], env=self.genv, check=True, capture_output=True).stdout
            self.keys[name] = (fpr, write(self.tmp / (name + ".asc"), asc))
        self.data = write(self.tmp / "q.tar.xz", b"tarball bytes")
        self.sig = self.tmp / "q.tar.xz.sig"
        subprocess.run(["gpg", "--batch", "--detach-sign", "--local-user", self.keys["official"][0], "-o", str(self.sig), str(self.data)],
                       env=self.genv, check=True, capture_output=True)

    def verify(self, key="official", allowed=None, data=None, sig=None):
        fpr, asc = self.keys[key]
        return kit.verify_signature(data or self.data, sig or self.sig, asc, [fpr] if allowed is None else allowed)

    def test_a_good_signature_by_a_listed_key_is_accepted_and_names_the_signer(self):
        self.assertEqual(self.verify(), self.keys["official"][0])

    def test_a_signature_by_an_unlisted_key_is_refused(self):
        with self.assertRaises(kit.KitRefused):
            self.verify(allowed=[self.keys["other"][0]])

    def test_a_key_file_that_is_not_the_listed_key_is_refused(self):
        with self.assertRaises(kit.KitRefused):
            kit.verify_signature(self.data, self.sig, self.keys["other"][1], [self.keys["official"][0]])

    def test_a_modified_file_is_refused(self):
        bad = write(self.tmp / "bad.tar.xz", b"tarball bytez")
        with self.assertRaises(kit.KitRefused):
            self.verify(data=bad)

    def test_a_missing_or_empty_signature_is_refused(self):
        with self.assertRaises(kit.KitRefused):
            self.verify(sig=self.tmp / "nothing.sig")
        with self.assertRaises(kit.KitRefused):
            self.verify(sig=write(self.tmp / "empty.sig", b""))

    def test_a_signature_by_another_key_with_the_listed_key_file_is_refused(self):
        other_sig = self.tmp / "o.sig"
        subprocess.run(["gpg", "--batch", "--detach-sign", "--local-user", self.keys["other"][0], "-o", str(other_sig), str(self.data)],
                       env=self.genv, check=True, capture_output=True)
        with self.assertRaises(kit.KitRefused):
            self.verify(sig=other_sig)

    def test_the_callers_keyring_is_not_consulted(self):
        os.environ["GNUPGHOME"] = str(self.home)
        self.addCleanup(os.environ.pop, "GNUPGHOME", None)
        with self.assertRaises(kit.KitRefused):
            kit.verify_signature(self.data, self.sig, self.keys["other"][1], [self.keys["other"][0]])


class CommittedKey(unittest.TestCase):
    def test_pin_file_parses_and_names_the_signer_and_the_signature(self):
        pin = kit.read_release_pin(kit.RELEASE_PIN)
        self.assertEqual(pin["signers"][0][0], "CEACC9E15534EBABB82D3FA03353C9CEF108B584")
        self.assertEqual(pin["signature"]["name"], "qemu-10.2.4.tar.xz.sig")
        self.assertRegex(pin["signature"]["sha256"], r"^[0-9a-f]{64}$")

    @unittest.skipUnless(HAVE_GPG, "gpg")
    def test_the_committed_key_file_is_the_listed_key(self):
        pin = kit.read_release_pin(kit.RELEASE_PIN)
        with tempfile.TemporaryDirectory() as d:
            out = subprocess.run(["gpg", "--batch", "--with-colons", "--show-keys", str(kit.RELEASE_KEYS)], env={**os.environ, "GNUPGHOME": d},
                                 check=True, capture_output=True, text=True).stdout
        fprs = re.findall(r"^fpr:+([0-9A-F]{40}):", out, re.M)
        self.assertIn(pin["signers"][0][0], fprs)

    def test_pin_file_with_a_bad_line_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            for text in ("signer nothex x\n", "signature a.sig sha256=abc size=1\n", "unknown line\n", ""):
                p = write(Path(d) / "p", text)
                with self.assertRaises(kit.KitRefused):
                    kit.read_release_pin(p)

    def test_supported_versions_match_the_tarball_pin(self):
        name = next(l.split()[0] for l in (ROOT / "scripts/bench/qemu.pin").read_text().splitlines() if l.strip() and not l.startswith("#"))
        self.assertEqual(name, "qemu-%s.tar.xz" % kit.SUPPORTED_QEMU[0])
        self.assertEqual(kit.read_release_pin(kit.RELEASE_PIN)["signature"]["name"], name + ".sig")


class SourceFiles(Tmp):
    @unittest.skipUnless(HAVE_GIT, "git")
    def test_only_tracked_files_of_the_shipped_paths_are_listed(self):
        k = kayfabe_tree(self.tmp / "k")
        write(k / "Cargo.toml", "[workspace]\n")
        write(k / "docs/internal.md", "x")
        git(k, "init", "-q")
        git(k, "add", "-A")
        git(k, "commit", "-q", "-m", "x")
        write(k / "qemu/hw/misc/kf3/untracked.c", "u")
        files = kit.source_files(k)
        self.assertIn("Cargo.toml", files)
        self.assertIn("qemu/hw/misc/kf3/kf3.c", files)
        self.assertNotIn("docs/internal.md", files)
        self.assertNotIn("qemu/hw/misc/kf3/untracked.c", files)
        self.assertEqual(files, sorted(files))

    def test_every_shipped_path_exists_in_the_repository(self):
        for rel in kit.SOURCE_PATHS:
            self.assertTrue((ROOT / rel).exists(), rel)

    def test_the_kit_ships_itself_and_what_it_imports(self):
        for rel in ("scripts/release/build_qemu.py", "scripts/release/qemu-release.pin", "scripts/release/qemu-release-keys.asc",
                    "scripts/ci/kf_trust.py", "scripts/ci/qemu_source.py", "scripts/bench/qemu.pin", "LICENSE-GPL", "Cargo.lock",
                    "rust-toolchain.toml", "traces/driver_matrix/drmnv.tsv"):
            self.assertTrue(any(rel == p or rel.startswith(p.rstrip("/") + "/") for p in kit.SOURCE_PATHS), rel)

    def test_no_generated_or_private_tree_is_shipped(self):
        for rel in kit.SOURCE_PATHS:
            for bad in ("target", "archive", "third_party", ".git", "traces/"):
                self.assertFalse(rel.startswith(bad) and rel != "traces/driver_matrix/drmnv.tsv", rel)


class SafeExtract(Tmp):
    def tar(self, members):
        p = self.tmp / "x.tar"
        with tarfile.open(p, "w") as tf:
            for info, data in members:
                tf.addfile(info, io.BytesIO(data) if data is not None else None)
        return p

    def info(self, name, size=0, type_=tarfile.REGTYPE, linkname=""):
        i = tarfile.TarInfo(name)
        i.size, i.type, i.linkname = size, type_, linkname
        return i

    def test_a_normal_archive_is_extracted(self):
        p = self.tar([(self.info("top/a.txt", 1), b"a")])
        kit.safe_extract(p, self.tmp / "out")
        self.assertEqual((self.tmp / "out/top/a.txt").read_text(), "a")

    def test_path_traversal_absolute_names_and_escaping_links_are_refused(self):
        for member in (self.info("../evil", 1), self.info("/abs/evil", 1), self.info("top/ln", 0, tarfile.SYMTYPE, "../../etc/passwd")):
            p = self.tar([(member, b"x" if member.size else None)])
            with self.assertRaises(kit.KitRefused):
                kit.safe_extract(p, self.tmp / "out")

    def test_device_nodes_are_refused(self):
        p = self.tar([(self.info("top/dev", 0, tarfile.CHRTYPE), None)])
        with self.assertRaises(kit.KitRefused):
            kit.safe_extract(p, self.tmp / "out")


class FakeRunner:
    """Records every command and creates what the real tool would, so the orchestration can be judged without a compiler."""

    def __init__(self, fail=None):
        self.calls = []
        self.fail = fail

    def __call__(self, argv, cwd, env, log):
        self.calls.append((list(argv), str(cwd), dict(env)))
        name = Path(argv[0]).name
        if self.fail == name:
            return 1
        if name == "cargo":
            target = Path(env["CARGO_TARGET_DIR"]) / "release"
            target.mkdir(parents=True, exist_ok=True)
            (target / "libkf_qemu.a").write_bytes(b"!<arch>\nrust")
        elif name == "configure":
            Path(cwd, "build.ninja").write_text("rules")
        elif name == "ninja" and "install" in argv:
            stage = Path(env["DESTDIR"]) / "usr/local"
            (stage / "bin").mkdir(parents=True, exist_ok=True)
            (stage / "bin/qemu-system-x86_64").write_bytes(b"ELF fake qemu")
            (stage / "share/qemu").mkdir(parents=True, exist_ok=True)
            (stage / "share/qemu/bios.bin").write_bytes(b"bios")
        return 0


class Orchestration(Tmp):
    def setUp(self):
        super().setUp()
        self.k = kayfabe_tree(self.tmp / "k")
        write(self.k / "SOURCE_INFO.json", json.dumps({"revision": "ab" * 20, "source_date_epoch": 1700000000}))
        for name in ("LICENSE", "LICENSE-APACHE", "LICENSE-GPL", "THIRD_PARTY_NOTICES.md"):
            write(self.k / name, name)
        self.q = qemu_tree(self.tmp / "q")
        write(self.q / "COPYING", "GPL")

    def build(self, work="w", out="o", runner=None, tools=None, **kw):
        runner = runner or FakeRunner()
        args = dict(kayfabe_root=self.k, qemu=kit.QemuInput.tree(self.q), out=self.tmp / out, workdir=self.tmp / work, jobs=2,
                    runner=runner, tool_versions=tools or (lambda env: {"cc": "gcc 1"}), caller_env={"PATH": "/usr/bin", "HOME": "/h"})
        args.update(kw)
        return kit.build(**args), runner

    def test_steps_run_in_order_and_the_binary_tarball_and_manifest_are_written(self):
        result, runner = self.build()
        names = [Path(c[0][0]).name for c in runner.calls]
        self.assertEqual(names, ["cargo", "configure", "ninja", "ninja"])
        self.assertIn("install", runner.calls[-1][0])
        out = Path(result["out"])
        self.assertTrue((out / result["assets"]["binary"]).is_file())
        manifest = json.loads((out / result["assets"]["manifest"]).read_text())
        self.assertEqual(manifest["schema"], kit.SCHEMA)
        self.assertEqual(manifest["qemu"]["version"], "10.2.4")
        self.assertEqual(manifest["kayfabe"]["revision"], "ab" * 20)
        self.assertEqual(manifest["kayfabe"]["kf3_abi"], 36)

    def test_cargo_is_locked_release_and_only_the_qemu_crate(self):
        _, runner = self.build()
        argv = runner.calls[0][0]
        self.assertEqual(argv[:2], ["cargo", "build"])
        for flag in ("--release", "--locked"):
            self.assertIn(flag, argv)
        self.assertEqual(argv[argv.index("-p") + 1], "kf-qemu")

    def test_configure_runs_in_the_build_directory_with_the_recorded_line(self):
        result, runner = self.build()
        argv, cwd, _ = runner.calls[1]
        self.assertTrue(argv[0].endswith("/src/configure"))
        self.assertTrue(cwd.endswith("/build"))
        manifest = json.loads((Path(result["out"]) / result["assets"]["manifest"]).read_text())
        self.assertEqual(manifest["configure"]["line"], kit.configure_line(argv[1:], str(self.tmp / "w/src"), str(self.tmp / "w/build"), str(self.k)))

    def test_install_goes_to_a_staging_directory_and_the_tarball_holds_only_the_install(self):
        result, runner = self.build()
        self.assertTrue(runner.calls[-1][2]["DESTDIR"].endswith("/stage"))
        with tarfile.open(Path(result["out"]) / result["assets"]["binary"]) as tf:
            names = tf.getnames()
        top = names[0]
        self.assertTrue(top.startswith("kayfabe-qemu-10.2.4-"))
        for expected in ("bin/qemu-system-x86_64", "share/qemu/bios.bin", "LICENSE-GPL", "COPYING", "README.txt"):
            self.assertIn(top + "/" + expected, names)

    def test_a_failing_step_stops_the_build_and_leaves_no_output(self):
        for failing, count in (("cargo", 1), ("configure", 2), ("ninja", 3)):
            with self.subTest(failing):
                runner = FakeRunner(fail=failing)
                with self.assertRaises(kit.BuildFailed):
                    self.build(work="w_" + failing, out="o_" + failing, runner=runner)
                self.assertEqual(len(runner.calls), count)
                self.assertFalse((self.tmp / ("o_" + failing)).exists() and any((self.tmp / ("o_" + failing)).iterdir()))

    def test_unsupported_qemu_is_refused_before_anything_runs(self):
        write(self.q / "VERSION", "9.0.0\n")
        runner = FakeRunner()
        with self.assertRaises(kit.KitRefused):
            self.build(runner=runner)
        self.assertEqual(runner.calls, [])

    def test_the_users_qemu_tree_is_never_modified(self):
        before = kit.tree_digest(self.q)
        self.build()
        self.assertEqual(kit.tree_digest(self.q), before)
        self.assertFalse((self.q / "hw/misc/kf3").exists())

    def test_two_builds_in_different_directories_give_identical_outputs(self):
        a, _ = self.build(work="w1", out="o1")
        b, _ = self.build(work="w2", out="o2")
        for key in ("binary", "manifest"):
            self.assertEqual((self.tmp / "o1" / a["assets"][key]).read_bytes(), (self.tmp / "o2" / b["assets"][key]).read_bytes())

    def test_no_build_path_appears_in_the_manifest(self):
        result, _ = self.build(work="needle_work", out="needle_out")
        text = (Path(result["out"]) / result["assets"]["manifest"]).read_text()
        for needle in ("needle_work", "needle_out", str(self.tmp)):
            self.assertNotIn(needle, text)

    def test_manifest_records_inputs_patches_and_output_hashes(self):
        result, _ = self.build()
        m = json.loads((Path(result["out"]) / result["assets"]["manifest"]).read_text())
        self.assertEqual(m["qemu"]["source"]["kind"], "tree")
        self.assertRegex(m["qemu"]["source"]["tree_sha256"], r"^[0-9a-f]{64}$")
        self.assertIn("hw/misc/kf3/kf3.c", [p["path"] for p in m["patches"]])
        self.assertEqual(m["environment"]["SOURCE_DATE_EPOCH"], "1700000000")
        names = {o["name"]: o for o in m["outputs"]}
        binary = Path(result["out"]) / result["assets"]["binary"]
        self.assertEqual(names[result["assets"]["binary"]]["sha256"], sha(binary.read_bytes()))
        self.assertEqual(names["bin/qemu-system-x86_64"]["sha256"], sha(b"ELF fake qemu"))
        self.assertEqual(m["toolchain"], {"cc": "gcc 1"})

    def test_the_qemu_tree_digest_is_taken_before_the_overlay(self):
        before = kit.tree_digest(self.q)
        result, _ = self.build()
        m = json.loads((Path(result["out"]) / result["assets"]["manifest"]).read_text())
        self.assertEqual(m["qemu"]["source"]["tree_sha256"], before)

    def test_a_dirty_revision_is_marked_in_the_asset_names_and_refused_unless_allowed(self):
        names = kit.asset_names("10.2.4", "ab" * 20)
        self.assertEqual(names["binary"], "kayfabe-qemu-10.2.4-abababab-linux-x86_64.tar.xz")
        self.assertEqual(names["source"], "kayfabe-qemu-10.2.4-abababab-source.tar.xz")
        self.assertEqual(names["manifest"], "kayfabe-qemu-10.2.4-abababab-manifest.json")
        self.assertEqual(kit.asset_names("10.2.4", "ab" * 20, dirty=True)["binary"], "kayfabe-qemu-10.2.4-abababab-dirty-linux-x86_64.tar.xz")

    def test_the_environment_given_to_every_step_is_the_scrubbed_one(self):
        _, runner = self.build()
        for argv, _cwd, env in runner.calls:
            self.assertEqual(env["SOURCE_DATE_EPOCH"], "1700000000")
            self.assertNotIn("CFLAGS", env)
            self.assertEqual(env["TZ"], "UTC")

    def test_an_existing_workdir_that_is_not_ours_is_refused(self):
        write(self.tmp / "w/precious.txt", "keep")
        with self.assertRaises(kit.KitRefused):
            self.build()
        self.assertTrue((self.tmp / "w/precious.txt").exists())

    def test_the_output_directory_must_not_already_hold_the_assets(self):
        self.build()
        with self.assertRaises(kit.KitRefused):
            self.build(work="w2")


class Compare(Tmp):
    def dirs(self, same=True):
        for name in ("a", "b"):
            write(self.tmp / name / "kayfabe-qemu-x-linux-x86_64.tar.xz", "bin")
            write(self.tmp / name / "kayfabe-qemu-x-manifest.json", json.dumps({"a": 1, "source_tarball": name}))
        if not same:
            write(self.tmp / "b" / "kayfabe-qemu-x-linux-x86_64.tar.xz", "different")

    def test_identical_outputs_have_no_differences(self):
        self.dirs()
        self.assertEqual(kit.compare_outputs(self.tmp / "a", self.tmp / "b", ignore_keys=("source_tarball",)), [])

    def test_a_changed_binary_is_reported_by_name(self):
        self.dirs(same=False)
        diff = kit.compare_outputs(self.tmp / "a", self.tmp / "b", ignore_keys=("source_tarball",))
        self.assertEqual(len(diff), 1)
        self.assertIn("linux-x86_64.tar.xz", diff[0])

    def test_the_ignored_key_is_ignored_only_when_asked(self):
        self.dirs()
        diff = kit.compare_outputs(self.tmp / "a", self.tmp / "b", ignore_keys=())
        self.assertEqual(len(diff), 1)
        self.assertIn("manifest", diff[0])

    def test_a_file_on_one_side_only_is_reported(self):
        self.dirs()
        write(self.tmp / "a" / "extra.txt", "x")
        self.assertTrue(any("extra.txt" in d for d in kit.compare_outputs(self.tmp / "a", self.tmp / "b", ignore_keys=("source_tarball",))))


class CommandLine(Tmp):
    def run_cli(self, *args):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try:
                rc = kit.main(list(args))
            except SystemExit as e:
                rc = e.code
        return rc, out.getvalue(), err.getvalue()

    def test_two_qemu_inputs_at_once_are_a_usage_error(self):
        rc, _, _ = self.run_cli("build", "--qemu-tree", str(self.tmp), "--qemu-tarball", str(self.tmp / "x"), "--out", str(self.tmp / "o"))
        self.assertEqual(rc, 2)

    def test_a_source_tarball_excludes_the_other_inputs(self):
        rc, _, _ = self.run_cli("build", "--source-tarball", str(self.tmp / "s"), "--qemu-tree", str(self.tmp), "--out", str(self.tmp / "o"))
        self.assertEqual(rc, 2)

    def test_a_refusal_is_named_on_stderr_and_exits_one(self):
        write(self.tmp / "q/VERSION", "1.0.0\n")
        rc, _, err = self.run_cli("plan", "--qemu-tree", str(self.tmp / "q"))
        self.assertEqual(rc, 1)
        self.assertIn("KIT_REFUSED", err)
        self.assertIn("10.2.4", err)

    def test_plan_prints_the_configure_line_and_the_inputs_and_builds_nothing(self):
        qemu_tree(self.tmp / "q")
        rc, out, _ = self.run_cli("plan", "--qemu-tree", str(self.tmp / "q"))
        self.assertEqual(rc, 0)
        self.assertIn("--target-list=x86_64-softmmu", out)
        self.assertIn("tree_sha256", out)
        self.assertIn("kf3.c", out)

    def test_the_help_names_the_three_ways_to_give_a_qemu(self):
        _, out, _ = self.run_cli("build", "--help")
        for flag in ("--qemu-tarball", "--qemu-tree", "--source-tarball"):
            self.assertIn(flag, out)


def workflow():
    return yaml.safe_load(WORKFLOW.read_text())


def steps(job):
    return workflow()["jobs"][job]["steps"]


class Workflow(unittest.TestCase):
    def text(self):
        return WORKFLOW.read_text()

    def test_it_runs_only_on_manual_dispatch(self):
        on = workflow().get("on", workflow().get(True))
        self.assertEqual(sorted(on), ["workflow_dispatch"])
        for forbidden in ("push:", "pull_request", "schedule:", "workflow_run:", "release:"):
            self.assertNotIn(forbidden, self.text().split("jobs:")[0])

    def test_top_level_permissions_grant_nothing(self):
        self.assertEqual(workflow()["permissions"], {})

    def test_every_action_is_pinned_to_a_commit(self):
        uses = re.findall(r"^\s*-?\s*uses:\s*(\S+)", self.text(), re.M)
        self.assertTrue(uses)
        for u in uses:
            self.assertRegex(u, r"^[\w.-]+/[\w./-]+@[0-9a-f]{40}$")

    def test_the_only_secret_is_the_default_token(self):
        t = self.text()
        self.assertNotRegex(t, r"secrets\.(?!GITHUB_TOKEN)")
        self.assertNotIn("secrets: inherit", t)
        self.assertNotRegex(t, r"VAST|R2_|CLOUDFLARE|WORKER_")

    def test_the_build_job_holds_the_attestation_permissions_and_nothing_else_writes(self):
        jobs = workflow()["jobs"]
        self.assertEqual(jobs["build"]["permissions"], {"contents": "read", "id-token": "write", "attestations": "write"})
        for name, job in jobs.items():
            perms = job.get("permissions", {})
            if name != "build":
                self.assertNotIn("id-token", perms, name)
                self.assertNotIn("attestations", perms, name)
        self.assertEqual(jobs["draft"]["permissions"], {"contents": "write"})

    def test_attestation_covers_the_binary_the_source_and_the_manifest_from_this_run(self):
        attest = [s for s in steps("build") if "attest-build-provenance" in str(s.get("uses", ""))]
        self.assertEqual(len(attest), 1)
        subjects = attest[0]["with"]["subject-path"]
        for needle in ("linux-x86_64.tar.xz", "source.tar.xz", "manifest.json"):
            self.assertIn(needle, subjects)

    def test_attestation_is_made_only_on_the_master_branch(self):
        attest = next(s for s in steps("build") if "attest-build-provenance" in str(s.get("uses", "")))
        self.assertIn("refs/heads/master", attest["if"])

    def test_the_release_is_a_draft_and_is_never_published(self):
        t = self.text()
        self.assertIn("--draft", t)
        for forbidden in ("--draft=false", "--latest", "gh release edit", "--prerelease=false", "gh release upload --clobber", "gh api -X PATCH", "draft: false"):
            self.assertNotIn(forbidden, t)
        self.assertEqual(len(re.findall(r"gh release create", t)), 1)

    def test_the_draft_job_needs_the_build_and_runs_only_on_master(self):
        draft = workflow()["jobs"]["draft"]
        self.assertIn("build", draft["needs"] if isinstance(draft["needs"], list) else [draft["needs"]])
        self.assertIn("refs/heads/master", draft["if"])

    def test_the_gate_check_is_the_same_one_the_binary_release_uses(self):
        t = self.text()
        self.assertIn("python3 -I ci/vast-broker/release_gate.py", t)
        self.assertIn("gate", workflow()["jobs"])
        needs = workflow()["jobs"]["build"].get("needs", [])
        self.assertIn("gate", needs if isinstance(needs, list) else [needs])

    def test_checkout_does_not_keep_credentials(self):
        for job in workflow()["jobs"].values():
            for s in job["steps"]:
                if "actions/checkout" in str(s.get("uses", "")):
                    self.assertIs(s["with"]["persist-credentials"], False)

    def test_the_build_checks_reproducibility_before_it_attests(self):
        names = [s.get("name", "") for s in steps("build")]
        repro = next(i for i, n in enumerate(names) if "reproduc" in n.lower())
        attest = next(i for i, n in enumerate(names) if "attest" in n.lower())
        self.assertLess(repro, attest)
        t = self.text()
        self.assertIn("build_qemu.py reproduce", t)

    def test_tag_input_is_validated_before_use(self):
        t = self.text()
        self.assertIn("inputs.tag", t)
        self.assertRegex(t, r"\^\[A-Za-z0-9")

    def test_the_workflow_inputs_reach_shell_only_through_the_environment(self):
        for job in workflow()["jobs"].values():
            for s in job["steps"]:
                self.assertNotIn("${{ inputs.", str(s.get("run", "")))
                self.assertNotIn("${{ github.event.inputs", str(s.get("run", "")))


if __name__ == "__main__":
    unittest.main()
