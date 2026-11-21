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

    def test_content_changes_the_digest(self):
        c = self.make("c")
        write(c / "a.txt", "alphA")
        self.assertNotEqual(kit.tree_digest(c), kit.tree_digest(self.make("base")))

    def test_a_rename_changes_the_digest(self):
        p = self.make("p")
        (p / "a.txt").rename(p / "z.txt")
        self.assertNotEqual(kit.tree_digest(p), kit.tree_digest(self.make("base")))

    def test_the_exec_bit_changes_the_digest(self):
        x = self.make("x")
        (x / "a.txt").chmod(0o755)
        self.assertNotEqual(kit.tree_digest(x), kit.tree_digest(self.make("base")))

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

    def dirty_tree(self):
        k = kayfabe_tree(self.tmp / "k")
        git(k, "init", "-q")
        git(k, "add", "-A")
        git(k, "commit", "-q", "-m", "x")
        write(k / "qemu/hw/misc/kf3/kf3.c", "changed\n")
        return k

    @unittest.skipUnless(HAVE_GIT, "git")
    def test_a_modified_tracked_file_is_dirty_when_allowed(self):
        self.assertTrue(kit.kayfabe_identity(self.dirty_tree(), allow_dirty=True)["dirty"])

    @unittest.skipUnless(HAVE_GIT, "git")
    def test_a_modified_tracked_file_is_refused_by_default(self):
        with self.assertRaises(kit.KitRefused):
            kit.kayfabe_identity(self.dirty_tree())

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

    def stamp_refused(self, text):
        k = kayfabe_tree(self.tmp / "k")
        write(k / "SOURCE_INFO.json", text)
        with self.assertRaises(kit.KitRefused):
            kit.kayfabe_identity(k)

    def test_a_stamp_with_a_short_revision_is_refused(self):
        self.stamp_refused('{"revision": "abc", "source_date_epoch": 1}')

    def test_a_stamp_with_a_string_epoch_is_refused(self):
        self.stamp_refused('{"revision": "%s", "source_date_epoch": "1"}' % ("a" * 40))

    def test_a_stamp_with_a_zero_epoch_is_refused(self):
        self.stamp_refused('{"revision": "%s", "source_date_epoch": 0}' % ("a" * 40))

    def test_a_stamp_with_a_boolean_epoch_is_refused(self):
        self.stamp_refused('{"revision": "%s", "source_date_epoch": true}' % ("a" * 40))

    def test_a_stamp_that_is_not_json_is_refused(self):
        self.stamp_refused("nope")


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

    def test_the_signer_is_returned_in_upper_case_hex_of_forty_digits(self):
        self.assertRegex(self.verify(), r"^[0-9A-F]{40}$")

    def test_a_key_file_with_two_keys_accepts_a_signature_by_the_listed_one(self):
        both = write(self.tmp / "both.asc", self.keys["official"][1].read_bytes() + self.keys["other"][1].read_bytes())
        self.assertEqual(kit.verify_signature(self.data, self.sig, both, [self.keys["official"][0]]), self.keys["official"][0])

    def test_a_key_file_with_two_keys_refuses_a_signature_by_the_unlisted_one(self):
        both = write(self.tmp / "both.asc", self.keys["official"][1].read_bytes() + self.keys["other"][1].read_bytes())
        other_sig = self.tmp / "o2.sig"
        subprocess.run(["gpg", "--batch", "--detach-sign", "--local-user", self.keys["other"][0], "-o", str(other_sig), str(self.data)],
                       env=self.genv, check=True, capture_output=True)
        with self.assertRaises(kit.KitRefused):
            kit.verify_signature(self.data, other_sig, both, [self.keys["official"][0]])

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
            kit.verify_signature(self.data, self.sig, self.keys["other"][1], [self.keys["official"][0], self.keys["other"][0]])

    def test_the_callers_keyring_directory_is_left_alone(self):
        empty = self.tmp / "callers-gnupg"
        empty.mkdir(mode=0o700)
        os.environ["GNUPGHOME"] = str(empty)
        self.addCleanup(os.environ.pop, "GNUPGHOME", None)
        self.verify()
        self.assertEqual(list(empty.iterdir()), [])

    def test_a_listed_signer_that_is_not_the_signing_key_is_refused(self):
        both = write(self.tmp / "both.asc", self.keys["official"][1].read_bytes() + self.keys["other"][1].read_bytes())
        with self.assertRaises(kit.KitRefused):
            kit.verify_signature(self.data, self.sig, both, [self.keys["other"][0]])


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

    GOOD_PIN = "signer " + "A" * 40 + " x\nsignature q.sig sha256=" + "0" * 64 + " size=3\n"

    def test_a_valid_pin_file_with_one_unknown_line_added_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(kit.read_release_pin(write(Path(d) / "ok", self.GOOD_PIN))["signature"]["size"], 3)
            with self.assertRaises(kit.KitRefused):
                kit.read_release_pin(write(Path(d) / "p", self.GOOD_PIN + "mystery line\n"))

    def test_a_valid_pin_file_with_a_malformed_signature_line_added_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(kit.KitRefused):
                kit.read_release_pin(write(Path(d) / "p", self.GOOD_PIN + "signature r.sig sha256=zz size=1\n"))

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

    def test_an_absolute_link_is_left_out_and_reported_and_the_rest_is_extracted(self):
        p = self.tar([(self.info("top/a.txt", 1), b"a"), (self.info("top/abs", 0, tarfile.SYMTYPE, "/opt/X11/include"), None)])
        self.assertEqual(kit.safe_extract(p, self.tmp / "out"), ["top/abs"])
        self.assertTrue((self.tmp / "out/top/a.txt").is_file())
        self.assertFalse(os.path.lexists(self.tmp / "out/top/abs"))

    def test_device_nodes_are_refused(self):
        p = self.tar([(self.info("top/dev", 0, tarfile.CHRTYPE), None)])
        with self.assertRaises(kit.KitRefused):
            kit.safe_extract(p, self.tmp / "out")


class FakeRunner:
    """Records every command and creates what the real tool would, so the orchestration can be judged without a compiler."""

    def __init__(self, fail=None, no_archive=False):
        self.calls = []
        self.fail = fail
        self.no_archive = no_archive

    def __call__(self, argv, cwd, env, log):
        self.calls.append((list(argv), str(cwd), dict(env)))
        name = Path(argv[0]).name
        if self.fail == name:
            return 1
        if name == "cargo" and argv[1] == "vendor":
            Path(argv[-1]).mkdir(parents=True, exist_ok=True)
        elif name == "cargo":
            target = Path(env["CARGO_TARGET_DIR"]) / "release"
            target.mkdir(parents=True, exist_ok=True)
            if not self.no_archive:
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


class OrchestrationBase(Tmp):
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



class Orchestration(OrchestrationBase):
    def test_steps_run_in_order_and_the_binary_tarball_and_manifest_are_written(self):
        result, runner = self.build()
        names = [Path(c[0][0]).name for c in runner.calls]
        self.assertEqual(names, ["cargo", "cargo", "configure", "ninja", "ninja"])
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
        argv = runner.calls[1][0]
        self.assertEqual(argv[:2], ["cargo", "build"])
        self.assertIn("--release", argv)
        self.assertIn("--locked", argv)
        self.assertEqual(argv[argv.index("-p") + 1], "kf-qemu")

    def test_configure_runs_in_the_build_directory_with_the_recorded_line(self):
        result, runner = self.build()
        argv, cwd, _ = runner.calls[2]
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

    def failing(self, step, count):
        runner = FakeRunner(fail=step)
        with self.assertRaises(kit.BuildFailed):
            self.build(runner=runner)
        self.assertEqual(len(runner.calls), count)
        self.assertFalse((self.tmp / "o").exists() and any((self.tmp / "o").iterdir()))

    def test_a_failing_cargo_stops_the_build_and_leaves_no_output(self):
        self.failing("cargo", 1)

    def test_a_failing_configure_stops_the_build_and_leaves_no_output(self):
        self.failing("configure", 3)

    def test_a_failing_compile_stops_the_build_and_leaves_no_output(self):
        self.failing("ninja", 4)

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

    def test_asset_names_carry_the_version_and_the_short_revision(self):
        names = kit.asset_names("10.2.4", "ab" * 20)
        self.assertEqual(names["binary"], "kayfabe-qemu-10.2.4-abababab-linux-x86_64.tar.xz")
        self.assertEqual(names["source"], "kayfabe-qemu-10.2.4-abababab-source.tar.xz")
        self.assertEqual(names["manifest"], "kayfabe-qemu-10.2.4-abababab-manifest.json")

    def test_a_dirty_revision_is_marked_in_the_asset_names(self):
        self.assertEqual(kit.asset_names("10.2.4", "ab" * 20, dirty=True)["binary"], "kayfabe-qemu-10.2.4-abababab-dirty-linux-x86_64.tar.xz")

    def test_the_environment_given_to_every_step_is_the_scrubbed_one(self):
        _, runner = self.build(caller_env={"PATH": "/usr/bin", "HOME": "/h", "CFLAGS": "-O0", "SOURCE_DATE_EPOCH": "7", "LD_PRELOAD": "/x.so"})
        for argv, _cwd, env in runner.calls:
            self.assertEqual(env["SOURCE_DATE_EPOCH"], "1700000000")
            self.assertNotIn("CFLAGS", env)
            self.assertNotIn("LD_PRELOAD", env)
            self.assertEqual(env["TZ"], "UTC")

    def test_cargo_runs_in_the_kayfabe_root_with_every_rust_path_remapped(self):
        _, runner = self.build()
        argv, cwd, env = runner.calls[1]
        self.assertEqual(cwd, str(self.k))
        self.assertIn("--remap-path-prefix=%s=/kayfabe-src" % self.k, env["RUSTFLAGS"])
        self.assertIn("--remap-path-prefix=/h/.cargo=/cargo-home", env["RUSTFLAGS"])

    def test_the_manifest_records_the_cargo_and_ninja_commands(self):
        result, _ = self.build()
        m = json.loads((Path(result["out"]) / result["assets"]["manifest"]).read_text())
        self.assertEqual(m["cargo"], ["build", "--release", "--locked", "-p", "kf-qemu"])

    def test_the_recorded_configure_line_has_no_gui_and_no_download(self):
        result, _ = self.build()
        m = json.loads((Path(result["out"]) / result["assets"]["manifest"]).read_text())
        line = m["configure"]["line"]
        tokens = line.split()
        for flag in ("--disable-gtk", "--disable-sdl", "--disable-download", "--enable-strip", "--disable-debug-info", "--target-list=x86_64-softmmu"):
            self.assertIn(flag, tokens)
        self.assertNotIn("--enable-gtk", tokens)
        self.assertIn("--with-pkgversion=kayfabe-abababab", tokens)
        self.assertIn("-ffile-prefix-map=<qemu-build>=/qemu-build", line)
        self.assertIn("-ffile-prefix-map=<kayfabe-src>=/kayfabe-src", line)

    def test_the_manifest_keys_are_sorted_at_every_level(self):
        result, _ = self.build()
        text = (Path(result["out"]) / result["assets"]["manifest"]).read_text()

        def check(node):
            if isinstance(node, dict):
                self.assertEqual(list(node), sorted(node))
                for v in node.values():
                    check(v)
            elif isinstance(node, list):
                for v in node:
                    check(v)
        check(json.loads(text, object_pairs_hook=dict))

    def test_a_tool_version_that_names_the_work_directory_is_refused(self):
        with self.assertRaises(kit.KitRefused):
            self.build(tools=lambda env: {"cc": "gcc in %s" % (self.tmp / "w")})

    def test_the_qemu_trees_git_directory_is_not_copied(self):
        write(self.q / ".git/HEAD", "ref: x")
        seen = []

        class Spy(FakeRunner):
            def __call__(self, argv, cwd, env, log):
                if Path(argv[0]).name == "configure":
                    seen.append((Path(argv[0]).parent / ".git").exists())
                return super().__call__(argv, cwd, env, log)

        self.build(runner=Spy())
        self.assertEqual(seen, [False])

    def test_the_qemu_trees_git_directory_is_not_part_of_the_recorded_digest(self):
        plain = kit.tree_digest(self.q)
        write(self.q / ".git/HEAD", "ref: x")
        result, _ = self.build()
        m = json.loads((Path(result["out"]) / result["assets"]["manifest"]).read_text())
        self.assertEqual(m["qemu"]["source"]["tree_sha256"], plain)

    def test_an_existing_workdir_that_is_not_ours_is_refused(self):
        write(self.tmp / "w/precious.txt", "keep")
        with self.assertRaises(kit.KitRefused):
            self.build()

    def test_an_existing_workdir_that_is_not_ours_keeps_its_files(self):
        write(self.tmp / "w/precious.txt", "keep")
        with contextlib.suppress(kit.KitRefused):
            self.build()
        self.assertTrue((self.tmp / "w/precious.txt").exists())

    def test_a_workdir_the_kit_made_is_replaced(self):
        self.build(out="o1")
        result, _ = self.build(out="o2")
        self.assertTrue((self.tmp / "o2" / result["assets"]["binary"]).is_file())

    def test_the_output_directory_must_not_already_hold_the_assets(self):
        self.build()
        with self.assertRaises(kit.KitRefused):
            self.build(work="w2")


class OrchestrationMore(OrchestrationBase):
    """The same fixture, more observations."""

    def manifest(self, result):
        return json.loads((Path(result["out"]) / result["assets"]["manifest"]).read_text())

    def test_a_version_that_extends_a_known_one_is_refused_by_the_build(self):
        write(self.q / "VERSION", "10.2.40\n")
        with self.assertRaises(kit.KitRefused):
            self.build()

    def test_a_version_file_with_extra_lines_is_refused_by_the_build(self):
        write(self.q / "VERSION", "10.2.4\nlocal changes\n")
        with self.assertRaises(kit.KitRefused):
            self.build()

    def test_the_recorded_tree_digest_changes_with_the_exec_bit(self):
        a, _ = self.build(work="w1", out="o1")
        (self.q / "configure").chmod(0o644)
        b, _ = self.build(work="w2", out="o2")
        self.assertNotEqual(self.manifest(a)["qemu"]["source"]["tree_sha256"], self.manifest(b)["qemu"]["source"]["tree_sha256"])

    def test_the_recorded_tree_digest_changes_with_a_renamed_file(self):
        a, _ = self.build(work="w1", out="o1")
        (self.q / "COPYING").rename(self.q / "COPYING.renamed")
        b, _ = self.build(work="w2", out="o2")
        self.assertNotEqual(self.manifest(a)["qemu"]["source"]["tree_sha256"], self.manifest(b)["qemu"]["source"]["tree_sha256"])

    def test_no_git_directory_is_copied_when_the_build_runs(self):
        write(self.q / ".git/HEAD", "ref: x")
        seen = []

        class Spy(FakeRunner):
            def __call__(self, argv, cwd, env, log):
                if Path(argv[0]).name == "ninja":
                    seen.append((Path(cwd).parent / "src/.git").exists())
                return super().__call__(argv, cwd, env, log)

        self.build(runner=Spy())
        self.assertEqual(seen, [False, False])

    def test_the_built_tree_has_both_hunks_and_the_device(self):
        self.build()
        src = self.tmp / "w/src"
        self.assertIn("subdir('kf3')", (src / "hw/misc/meson.build").read_text())
        self.assertIn("config KF3", (src / "hw/misc/Kconfig").read_text())
        self.assertTrue((src / "hw/misc/kf3/libkf_qemu.a").is_file())

    def test_a_foreign_file_in_the_qemu_tree_stops_the_build(self):
        write(self.q / "hw/misc/kf3/mine.c", "x")
        runner = FakeRunner()
        with self.assertRaises(kit.KitRefused):
            self.build(runner=runner)
        self.assertEqual(len(runner.calls), 2)

    def test_a_cargo_run_that_leaves_no_archive_stops_the_build(self):
        runner = FakeRunner(no_archive=True)
        with self.assertRaises(kit.KitRefused):
            self.build(runner=runner)
        self.assertEqual(len(runner.calls), 2)

    def test_crates_are_vendored_first_and_cargo_is_told_to_use_them(self):
        _, runner = self.build()
        vendor, build = runner.calls[0][0], runner.calls[1][0]
        self.assertEqual(vendor[:3], ["cargo", "vendor", "--locked"])
        self.assertTrue(vendor[-1].endswith("/w/vendor"))
        self.assertIn('source.crates-io.replace-with="vendored-sources"', build)
        self.assertIn('source.vendored-sources.directory="%s"' % vendor[-1], build)
        self.assertIn("--remap-path-prefix=%s=/kayfabe-src/vendor" % vendor[-1], runner.calls[1][2]["RUSTFLAGS"])

    def test_a_source_tree_that_carries_its_vendor_directory_is_not_vendored_again(self):
        write(self.k / "vendor/libc/x.rs", "x")
        write(self.k / ".cargo/config.toml", "[source]\n")
        _, runner = self.build()
        self.assertEqual([c[0][1] for c in runner.calls if c[0][0] == "cargo"], ["build"])
        self.assertNotIn("--config", runner.calls[0][0])

    def test_the_manifest_lists_the_two_hunks(self):
        m = self.manifest(self.build()[0])
        kinds = {p["path"]: p["kind"] for p in m["patches"]}
        self.assertEqual(kinds["hw/misc/Kconfig"], "append-stanza")
        self.assertEqual(kinds["hw/misc/meson.build"], "append-line")
        self.assertEqual(len(m["patches"]), 7)

    def test_symlinks_stay_symlinks_in_the_binary_tarball(self):
        class Linking(FakeRunner):
            def __call__(self, argv, cwd, env, log):
                rc = super().__call__(argv, cwd, env, log)
                if Path(argv[0]).name == "ninja" and "install" in argv:
                    os.symlink("bios.bin", Path(env["DESTDIR"]) / "usr/local/share/qemu/bios-alias.bin")
                return rc

        result, _ = self.build(runner=Linking())
        with tarfile.open(Path(result["out"]) / result["assets"]["binary"]) as tf:
            link = next(m for m in tf.getmembers() if m.name.endswith("bios-alias.bin"))
        self.assertTrue(link.issym())

    def test_no_member_of_the_binary_tarball_names_an_owner(self):
        result, _ = self.build()
        with tarfile.open(Path(result["out"]) / result["assets"]["binary"]) as tf:
            self.assertEqual({(m.uid, m.gid, m.uname, m.gname) for m in tf.getmembers()}, {(0, 0, "", "")})

    def test_all_licence_texts_are_in_the_binary_tarball(self):
        result, _ = self.build()
        with tarfile.open(Path(result["out"]) / result["assets"]["binary"]) as tf:
            names = {Path(n).name for n in tf.getnames()}
        for name in ("LICENSE", "LICENSE-APACHE", "LICENSE-GPL", "THIRD_PARTY_NOTICES.md", "COPYING"):
            self.assertIn(name, names)

    def test_a_refused_second_build_leaves_the_first_outputs_untouched(self):
        a, _ = self.build()
        before = (self.tmp / "o" / a["assets"]["binary"]).read_bytes()
        with self.assertRaises(kit.KitRefused):
            self.build(work="w2")
        self.assertEqual((self.tmp / "o" / a["assets"]["binary"]).read_bytes(), before)

    def tarball_input(self, name="qemu-10.2.4.tar.xz"):
        src = qemu_tree(self.tmp / "tb/qemu-10.2.4")
        write(src / "COPYING", "GPL")
        os.symlink("/opt/X11/include", src / "abs")
        kit.pack_tree(self.tmp / "tb/qemu-10.2.4", self.tmp / name, 1, "qemu-10.2.4")
        return kit.QemuInput.tarball(self.tmp / name)

    def test_a_tarball_input_is_extracted_and_recorded_with_its_hash(self):
        qi = self.tarball_input()
        result, _ = self.build(qemu=qi)
        src = self.manifest(result)["qemu"]["source"]
        self.assertEqual((src["kind"], src["name"], src["anchors"]), ("tarball", "qemu-10.2.4.tar.xz", ["user"]))
        self.assertEqual(src["sha256"], sha((self.tmp / "qemu-10.2.4.tar.xz").read_bytes()))

    def test_the_dropped_absolute_link_is_recorded_in_the_manifest(self):
        result, _ = self.build(qemu=self.tarball_input())
        self.assertEqual(self.manifest(result)["qemu"]["source"]["dropped_absolute_links"], ["qemu-10.2.4/abs"])
        self.assertFalse(os.path.lexists(self.tmp / "w/src/abs"))

    def test_a_tarball_with_two_top_level_directories_is_refused(self):
        write(self.tmp / "tb2/a/VERSION", "10.2.4\n")
        write(self.tmp / "tb2/b/VERSION", "10.2.4\n")
        kit.pack_tree(self.tmp / "tb2", self.tmp / "qemu-10.2.4.tar.xz", 1, "x")
        with self.assertRaises(kit.KitRefused):
            self.build(qemu=kit.QemuInput.tarball(self.tmp / "qemu-10.2.4.tar.xz"))


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

    def test_a_dotted_key_names_a_nested_value(self):
        for name, anchors in (("a", ["x"]), ("b", ["x", "y"])):
            write(self.tmp / name / "kayfabe-qemu-x-manifest.json", json.dumps({"qemu": {"source": {"anchors": anchors, "sha256": "1"}}}))
            write(self.tmp / name / "kayfabe-qemu-x-linux-x86_64.tar.xz", "bin")
        self.assertEqual(kit.compare_outputs(self.tmp / "a", self.tmp / "b", ignore_keys=("qemu.source.anchors",)), [])
        self.assertEqual(len(kit.compare_outputs(self.tmp / "a", self.tmp / "b", ignore_keys=())), 1)

    def test_a_changed_binary_is_reported_by_name(self):
        self.dirs(same=False)
        diff = kit.compare_outputs(self.tmp / "a", self.tmp / "b", ignore_keys=("source_tarball",))
        self.assertEqual(len(diff), 1)
        self.assertIn("linux-x86_64.tar.xz", diff[0])

    def test_a_differing_key_is_reported_when_nothing_is_ignored(self):
        self.dirs()
        diff = kit.compare_outputs(self.tmp / "a", self.tmp / "b", ignore_keys=())
        self.assertEqual(len(diff), 1)
        self.assertIn("manifest", diff[0])

    def test_only_the_named_key_is_ignored(self):
        self.dirs()
        write(self.tmp / "b" / "kayfabe-qemu-x-manifest.json", json.dumps({"a": 2, "source_tarball": "b"}))
        self.assertEqual(len(kit.compare_outputs(self.tmp / "a", self.tmp / "b", ignore_keys=("source_tarball",))), 1)

    def test_a_file_on_the_other_side_only_is_reported_too(self):
        self.dirs()
        write(self.tmp / "b" / "extra.txt", "x")
        self.assertTrue(any("only in b" in d for d in kit.compare_outputs(self.tmp / "a", self.tmp / "b", ignore_keys=("source_tarball",))))

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

    def test_a_build_of_an_unsupported_tree_exits_one(self):
        write(self.tmp / "q/VERSION", "1.0.0\n")
        rc, _, err = self.run_cli("build", "--qemu-tree", str(self.tmp / "q"), "--out", str(self.tmp / "o"), "--workdir", str(self.tmp / "w"))
        self.assertEqual(rc, 1)
        self.assertIn("KIT_REFUSED", err)

    def test_plan_refuses_a_version_file_with_extra_lines(self):
        write(self.tmp / "q/VERSION", "10.2.4\nextra\n")
        rc, _, _ = self.run_cli("plan", "--qemu-tree", str(self.tmp / "q"))
        self.assertEqual(rc, 1)

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

    def test_each_job_holds_only_the_permissions_of_its_role(self):
        jobs = workflow()["jobs"]
        self.assertEqual(jobs["build"]["permissions"], {"contents": "read"})
        self.assertEqual(jobs["attest"]["permissions"], {"id-token": "write", "attestations": "write"})
        self.assertEqual(jobs["draft"]["permissions"], {"contents": "write", "attestations": "read"})
        self.assertNotIn("id-token", jobs["draft"]["permissions"])
        self.assertNotIn("id-token", jobs["gate"].get("permissions", {}))

    def test_attestation_covers_the_binary_the_source_and_the_manifest_from_this_run(self):
        attest = [s for s in steps("attest") if "attest-build-provenance" in str(s.get("uses", ""))]
        self.assertEqual(len(attest), 1)
        subjects = attest[0]["with"]["subject-path"]
        for needle in ("linux-x86_64.tar.xz", "source.tar.xz", "manifest.json"):
            self.assertIn(needle, subjects)

    def test_attestation_is_made_only_on_the_master_branch(self):
        self.assertIn("github.ref == 'refs/heads/master'", workflow()["jobs"]["attest"]["if"])
        for s in steps("build"):
            self.assertNotIn("attest-build-provenance", str(s.get("uses", "")))

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

    def test_the_build_checks_reproducibility_and_the_attest_job_waits_for_it(self):
        names = [s.get("name", "").lower() for s in steps("build")]
        self.assertTrue(any("reproduc" in n for n in names))
        self.assertIn("build", workflow()["jobs"]["attest"]["needs"])
        self.assertIn("build_qemu.py reproduce", self.text())

    def test_no_trigger_can_run_without_a_human(self):
        head = self.text().split("\npermissions:")[0]
        triggers = re.findall(r"^  ([a-z_]+):", head.split("\non:\n")[1], re.M)
        self.assertEqual(triggers, ["workflow_dispatch"])

    def test_release_create_names_the_draft_flag_and_the_target_sha(self):
        line = next(l for l in self.text().splitlines() if "gh release create" in l)
        self.assertIn(" --draft ", line)
        self.assertIn('--target "$GITHUB_SHA"', line)

    def test_attestation_needs_the_gate_and_the_build_to_have_succeeded(self):
        cond = workflow()["jobs"]["attest"]["if"]
        self.assertIn("needs.gate.result == 'success'", cond)
        self.assertIn("needs.build.result == 'success'", cond)

    def test_only_the_draft_job_can_write_contents(self):
        for name, job in workflow()["jobs"].items():
            if name != "draft":
                self.assertNotEqual(job.get("permissions", {}).get("contents"), "write", name)

    def test_the_pins_are_the_ones_the_repo_already_uses(self):
        others = "".join(p.read_text() for p in (ROOT / ".github/workflows").glob("*.yml") if p.name != "release-qemu.yml")
        for use in re.findall(r"^\s*-?\s*uses:\s*(\S+)", self.text(), re.M):
            self.assertIn(use, others, use)

    def test_no_step_reads_the_secrets_context(self):
        for job in workflow()["jobs"].values():
            for s in job["steps"]:
                self.assertNotIn("secrets.", json.dumps(s))

    def test_published_assets_come_from_the_checked_build(self):
        run = next(s["run"] for s in steps("build") if "reproduc" in s.get("name", "").lower())
        self.assertIn('mv "$RUNNER_TEMP"/repro/a/* "$RUNNER_TEMP/assets/"', run)
        self.assertIn('--out "$RUNNER_TEMP/repro"', run)
        self.assertIn("--source-tarball", run)

    def test_every_job_that_publishes_or_attests_is_master_only(self):
        jobs = workflow()["jobs"]
        for name in ("gate", "attest", "draft"):
            self.assertIn("refs/heads/master", jobs[name]["if"], name)
        for name, job in jobs.items():
            if "gh release create" in json.dumps(job):
                self.assertIn("refs/heads/master", job["if"], name)

    def test_the_build_waits_for_the_gate_but_survives_its_skip_off_master(self):
        build = workflow()["jobs"]["build"]
        self.assertEqual(build["needs"], "gate")
        self.assertIn("github.ref != 'refs/heads/master'", build["if"])

    def test_the_job_conditions_are_exactly_these(self):
        jobs = workflow()["jobs"]
        self.assertEqual(jobs["build"]["if"], "${{ !cancelled() && (github.ref != 'refs/heads/master' || needs.gate.result == 'success') }}")
        self.assertEqual(jobs["attest"]["if"], "${{ github.ref == 'refs/heads/master' && needs.gate.result == 'success' && needs.build.result == 'success' }}")
        self.assertEqual(jobs["draft"]["if"], "${{ github.ref == 'refs/heads/master' && needs.gate.result == 'success' && needs.attest.result == 'success' }}")
        self.assertEqual(jobs["gate"]["if"], "${{ github.ref == 'refs/heads/master' }}")

    def test_on_master_the_build_needs_a_successful_gate_not_just_a_non_failed_one(self):
        cond = workflow()["jobs"]["build"]["if"]
        self.assertIn("needs.gate.result == 'success'", cond)
        self.assertNotIn("!= 'failure'", cond)
        self.assertIn("!cancelled()", cond)

    def test_tag_is_validated_in_the_job_that_creates_the_release(self):
        run = next(s["run"] for s in steps("draft") if "gh release create" in s.get("run", ""))
        self.assertRegex(run, r"\^\[A-Za-z0-9")

    def test_no_job_has_write_all(self):
        self.assertNotIn("write-all", self.text())
        self.assertNotIn("read-all", self.text())

    def test_the_reproducibility_step_runs_the_reproduce_command(self):
        run = next(s["run"] for s in steps("build") if "reproduc" in s.get("name", "").lower())
        self.assertIn("build_qemu.py reproduce", run)

    def test_the_source_tarball_is_made_before_the_reproducibility_check(self):
        names = [s.get("name", "").lower() for s in steps("build")]
        self.assertLess(next(i for i, n in enumerate(names) if n.startswith("source tarball")), next(i for i, n in enumerate(names) if "reproduc" in n))

    def test_the_draft_step_checks_the_tag_before_it_calls_gh(self):
        run = next(s["run"] for s in steps("draft") if "gh release create" in s.get("run", ""))
        self.assertLess(run.index("A-Za-z0-9"), run.index("gh "))

    def test_release_create_uses_the_tag_variable(self):
        self.assertIn('gh release create "$TAG"', self.text())

    def test_checkout_in_the_build_job_has_no_credentials(self):
        checkout = next(s for s in steps("build") if "actions/checkout" in str(s.get("uses", "")))
        self.assertIs(checkout["with"]["persist-credentials"], False)

    def test_the_source_tarball_is_attested(self):
        attest = next(s for s in steps("attest") if "attest-build-provenance" in str(s.get("uses", "")))
        self.assertIn("-source.tar.xz", attest["with"]["subject-path"])

    def test_the_manifest_is_attested(self):
        attest = next(s for s in steps("attest") if "attest-build-provenance" in str(s.get("uses", "")))
        self.assertIn("-manifest.json", attest["with"]["subject-path"])

    def test_the_build_job_has_no_oidc_and_no_attestation_step(self):
        build = workflow()["jobs"]["build"]
        self.assertNotIn("id-token", build["permissions"])
        self.assertNotIn("attestations", build["permissions"])
        self.assertNotIn("id-token", json.dumps(build))

    def test_the_attest_job_runs_no_checkout_and_no_build_code(self):
        for s in steps("attest"):
            text = json.dumps(s)
            for forbidden in ("actions/checkout", "build_qemu", "cargo", "apt-get", "ninja", "setup-"):
                self.assertNotIn(forbidden, text)

    def test_the_attest_job_rechecks_every_file_against_the_build_jobs_checksums(self):
        run = next(s["run"] for s in steps("attest") if "sha256sum" in s.get("run", ""))
        self.assertIn("sha256sum --check --strict SHA256SUMS", run)
        self.assertIn('[ "$(sha256sum SHA256SUMS | cut -d\' \' -f1)" = "$SUMS" ]', run)
        env = next(s["env"] for s in steps("attest") if "sha256sum" in s.get("run", ""))
        self.assertEqual(env["SUMS"], "${{ needs.build.outputs.sums }}")
        self.assertIn("sums", workflow()["jobs"]["build"]["outputs"])

    def test_the_attest_job_refuses_files_the_checksums_do_not_list(self):
        run = next(s["run"] for s in steps("attest") if "sha256sum" in s.get("run", ""))
        self.assertIn('[ "$listed" = "$present" ]', run)

    def test_the_attest_recheck_runs_before_the_attestation_and_has_all_three_checks(self):
        names = [s.get("name", "") for s in steps("attest")]
        self.assertLess(next(i for i, n in enumerate(names) if n.startswith("Every file")), next(i for i, n in enumerate(names) if n.startswith("Attest")))
        step = next(s for s in steps("attest") if s.get("name", "").startswith("Every file"))
        for needle in ("--check --strict", '= "$SUMS"', '"$listed" = "$present"'):
            self.assertIn(needle, step["run"])
        self.assertEqual(step["env"]["SUMS"], "${{ needs.build.outputs.sums }}")

    def test_the_build_job_publishes_the_digest_of_its_checksum_file(self):
        build = workflow()["jobs"]["build"]
        self.assertEqual(build["outputs"]["sums"], "${{ steps.sums.outputs.sums }}")
        step = next(s for s in build["steps"] if s.get("id") == "sums")
        self.assertIn("SHA256SUMS", step["run"])

    def draft_run(self):
        return next(s["run"] for s in steps("draft") if "gh release create" in s.get("run", ""))

    def test_the_draft_looks_for_an_existing_tag_before_it_creates_anything(self):
        run = self.draft_run()
        self.assertIn("git/ref/tags/$TAG", run)
        self.assertLess(run.index("git/ref/tags/$TAG"), run.index("gh release create"))

    def test_the_draft_names_the_error_for_an_existing_tag(self):
        self.assertIn('echo "the tag $TAG already exists"; exit 1', self.draft_run())

    def test_the_draft_looks_for_an_existing_release_before_it_creates_one(self):
        run = self.draft_run()
        self.assertIn("gh release list", run)
        self.assertLess(run.index("gh release list"), run.index("gh release create"))

    def test_the_draft_names_the_error_for_an_existing_release(self):
        self.assertIn('echo "a release for $TAG already exists"; exit 1', self.draft_run())

    def test_the_draft_job_may_read_attestations(self):
        self.assertEqual(workflow()["jobs"]["draft"]["permissions"].get("attestations"), "read")

    def test_no_release_command_uploads_by_glob(self):
        self.assertNotRegex(self.text(), r"gh release (create|upload)[^\n]* -- \*")

    def test_the_draft_uploads_only_the_four_verified_files(self):
        run = next(s["run"] for s in steps("draft") if "gh release create" in s.get("run", ""))
        self.assertIn('-- "${upload[@]}"', run)
        self.assertNotIn("-- *", run)
        self.assertIn('upload+=("$f")', run)

    def test_the_draft_refuses_an_unverified_extra_file(self):
        self.assertIn('[ "$(ls -A | wc -l)" -eq 4 ]', self.draft_run())

    def test_the_draft_names_the_error_for_an_unverified_extra_file(self):
        self.assertIn("the artifact holds files that were not verified", self.draft_run())

    def test_each_upload_pattern_must_match_exactly_one_file(self):
        self.assertIn('expected exactly one file for $f', self.draft_run())

    def test_the_reproducibility_check_runs_outside_the_checkout(self):
        t = self.text()
        self.assertNotIn("dist/", t)
        self.assertIn('--out "$RUNNER_TEMP/repro"', t)
        self.assertIn('--out "$RUNNER_TEMP/source"', t)

    def test_tag_input_is_validated_before_use(self):
        t = self.text()
        self.assertIn("inputs.tag", t)
        self.assertRegex(t, r"\^\[A-Za-z0-9")

    def test_the_workflow_inputs_reach_shell_only_through_the_environment(self):
        for job in workflow()["jobs"].values():
            for s in job["steps"]:
                self.assertNotIn("${{ inputs.", str(s.get("run", "")))
                self.assertNotIn("${{ github.event.inputs", str(s.get("run", "")))


HOWTO = ROOT / "docs/howto/building.md"
PIN_SIGNER = re.compile(r"--signer-workflow\s+(?:reindertpelsma/kayfabe|\"?\$\{?GITHUB_REPOSITORY\}?\"?|\$\{GITHUB_REPOSITORY\})/\.github/workflows/release-qemu\.yml")
PIN_REF = re.compile(r"--source-ref\s+refs/heads/master(?=\s|$|\")")
PIN_HOSTED = re.compile(r"--deny-self-hosted-runners(?=\s|$|\")")


def verify_commands(text):
    """Every `gh attestation verify` command of a text, with its backslash continuations joined."""
    joined = re.sub(r"\\\n\s*", " ", text)
    return [line for line in joined.splitlines() if "gh attestation verify" in line and not line.lstrip().startswith(("`", "-", "|"))]


class VerifyPins(unittest.TestCase):
    """A verification that names only the repository accepts an attestation from any workflow on any branch."""

    def pins(self, command):
        return {"signer-workflow": bool(PIN_SIGNER.search(command)), "source-ref": bool(PIN_REF.search(command)),
                "deny-self-hosted-runners": bool(PIN_HOSTED.search(command))}

    def assert_pinned(self, command, where):
        self.assertEqual(self.pins(command), {"signer-workflow": True, "source-ref": True, "deny-self-hosted-runners": True}, where + ": " + command)

    def test_the_helper_sees_a_missing_pin(self):
        self.assertFalse(self.pins("gh attestation verify f --repo r --source-ref refs/heads/master")["signer-workflow"])
        self.assertFalse(self.pins("gh attestation verify f --repo r --signer-workflow reindertpelsma/kayfabe/.github/workflows/release-qemu.yml")["source-ref"])
        self.assertFalse(self.pins("gh attestation verify f --repo r --source-ref refs/heads/main")["source-ref"])

    def test_the_draft_loop_verifies_with_all_three_pins(self):
        run = next(s["run"] for s in steps("draft") if "gh attestation verify" in s.get("run", ""))
        commands = [c for c in verify_commands(run) if c.lstrip().startswith("gh attestation verify")]
        self.assertEqual(len(commands), 1)
        self.assert_pinned(commands[0], "draft loop")

    def test_the_draft_loop_runs_over_the_four_attested_file_patterns(self):
        run = next(s["run"] for s in steps("draft") if "gh attestation verify" in s.get("run", ""))
        loop = re.search(r"for f in ([^;]+); do", run).group(1).split()
        self.assertEqual(loop, ["*-linux-x86_64.tar.xz", "*-source.tar.xz", "*-manifest.json", "SHA256SUMS"])

    def test_the_release_notes_command_has_all_three_pins(self):
        notes = [c for c in verify_commands(WORKFLOW.read_text()) if "Verify a download" in c]
        self.assertEqual(len(notes), 1)
        self.assert_pinned(notes[0], "release notes")

    def test_every_verify_command_of_the_workflow_has_all_three_pins(self):
        commands = verify_commands(WORKFLOW.read_text())
        self.assertGreaterEqual(len(commands), 2)
        for c in commands:
            self.assert_pinned(c, "workflow")

    def test_the_manual_command_has_all_three_pins(self):
        commands = verify_commands(HOWTO.read_text())
        self.assertEqual(len(commands), 1)
        self.assert_pinned(commands[0], "building.md")

    def test_every_verify_command_of_every_manual_page_has_all_three_pins(self):
        found = 0
        for page in sorted((ROOT / "docs/howto").rglob("*.md")):
            for c in verify_commands(page.read_text()):
                found += 1
                self.assert_pinned(c, str(page.relative_to(ROOT)))
        self.assertGreaterEqual(found, 1)

    def test_the_design_note_for_a_future_verifier_names_the_three_pins(self):
        text = (ROOT / "docs/design/V3_PACKAGING_AND_DISTRIBUTION.md").read_text()
        for needle in ("signer workflow path", "refs/heads/master", "--deny-self-hosted-runners"):
            self.assertIn(needle, text)


if __name__ == "__main__":
    unittest.main()
