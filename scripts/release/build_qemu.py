#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Build kit for a QEMU that has the kayfabe `kf3-gpu` device. STATUS: LIVE, 2026-11-21.

The kit lays the device (qemu/hw/misc/kf3/) and the Rust archive it links (crates/kf-qemu) into a QEMU source tree, configures,
builds and packs the result, and writes a manifest that is a function of the inputs only: two builds of the same inputs, in any
two directories on the same machine, give the same bytes. It installs nothing and takes no lock.

  build_qemu.py plan      [QEMU INPUT]               what would be built: inputs, patch list, the exact configure line
  build_qemu.py build     [QEMU INPUT] --out DIR     binary tarball + manifest in DIR
  build_qemu.py source    [QEMU INPUT] --out DIR     the corresponding-source tarball of a build (GPL), vendored crates included
  build_qemu.py reproduce [QEMU INPUT] --out DIR     build twice into DIR/a and DIR/b and fail unless the outputs are identical

QEMU INPUT is one of (none = the pinned official release, downloaded, signature-verified):
  --qemu-tarball FILE   a QEMU release tarball you have (verified against FILE.sig when that exists)
  --qemu-tree DIR       a QEMU source tree you have (for example the one a distribution builds from); never modified, .git ignored
  --source-tarball FILE a source tarball made by `source` (carries kayfabe, the vendored crates and the signed QEMU tarball)

Trust. The official QEMU tarball is accepted only if its detached qemu.org signature verifies against a signer listed in
qemu-release.pin (gpg, a private keyring built from qemu-release-keys.asc); the sha256 in scripts/bench/qemu.pin is a cache key
that kf_trust.py checks as well. A tarball or tree you supply is yours: it is recorded in the manifest as such. Nothing is
installed or run outside the work directory. Exit status: 0 ok, 1 refused (KIT_REFUSED), 2 usage, 3 a build step failed.
"""
import argparse
import contextlib
import dataclasses
import hashlib
import io
import json
import lzma
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
RELEASE_PIN = HERE / "qemu-release.pin"
RELEASE_KEYS = HERE / "qemu-release-keys.asc"

SCHEMA = "kayfabe.qemu-build/1"
ASSET_PREFIX = "kayfabe-qemu"
SUPPORTED_QEMU = ("10.2.4",)
WORK_MARK = ".kayfabe-qemu-work"
PACKAGE_NAME = "qemu-system-x86_64"

# What the source tarball carries. Every path is read by the build or is the kit itself; the licence texts travel with it.
SOURCE_PATHS = (
    "Cargo.toml", "Cargo.lock", "rust-toolchain.toml",
    "crates", "firmware", "cuda", "qemu", "tools/legal",
    "traces/driver_matrix/drmnv.tsv",
    "scripts/release", "scripts/ci/kf_trust.py", "scripts/ci/kf_fetch.py", "scripts/ci/qemu_source.py",
    "scripts/ci/trust_manifest.json", "scripts/bench/qemu.pin",
    "LICENSE", "LICENSE-APACHE", "LICENSE-GPL", "THIRD_PARTY_NOTICES.md", "README.md", "docs/howto/building.md",
)
DOC_FILES = ("LICENSE", "LICENSE-APACHE", "LICENSE-GPL", "THIRD_PARTY_NOTICES.md")

CONFIGURE_FLAGS = (
    "--target-list=x86_64-softmmu", "--enable-system", "--enable-kvm", "--without-default-features",
    "--disable-docs", "--disable-tools", "--disable-guest-agent", "--disable-werror",
    "--enable-slirp", "--enable-vnc", "--enable-pixman", "--enable-zstd", "--enable-tpm",
    "--disable-gtk", "--disable-sdl", "--disable-curses", "--disable-libssh", "--disable-vde",
    "--disable-download", "--disable-debug-info", "--enable-strip",
)
PASS_ENV = ("PATH", "HOME", "TMPDIR", "RUSTUP_TOOLCHAIN", "RUSTUP_HOME", "CARGO_HOME", "PKG_CONFIG_PATH", "PKG_CONFIG_LIBDIR",
            "CARGO_NET_OFFLINE", "SSL_CERT_FILE", "SSL_CERT_DIR", "http_proxy", "https_proxy", "no_proxy", "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY")
LIBS_PROBED = ("glib-2.0", "pixman-1", "slirp", "libzstd")
MAP_SRC, MAP_BUILD, MAP_KAYFABE = "/qemu-src", "/qemu-build", "/kayfabe-src"
MAP_CARGO, MAP_RUSTUP, MAP_TARGET = "/cargo-home", "/rustup-home", "/cargo-target"

CARGO_ARGS = ("build", "--release", "--locked", "-p", "kf-qemu")
MESON_HUNK = "subdir('kf3')"
KCONFIG_STANZA = "\nconfig KF3\n    bool\n    default y if TEST_DEVICES\n    depends on PCI\n"
DEVICE_DIR = "qemu/hw/misc/kf3"


class KitRefused(Exception):
    """An input or state the kit will not build from; the message names why."""


class BuildFailed(Exception):
    """A build step exited non-zero; the message carries the tail of its log."""


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


# ------------------------------------------------------------------------------------------------ configure line
def configure_args(*, src, build, kroot, pkgversion):
    """The arguments of QEMU's configure. Every directory the compilers can see is mapped to a fixed name."""
    maps = [(str(src), MAP_SRC), (str(build), MAP_BUILD), (str(kroot), MAP_KAYFABE), (os.path.relpath(src, build), MAP_SRC)]
    cflags = " ".join("-ffile-prefix-map=%s=%s" % pair for pair in maps)
    return list(CONFIGURE_FLAGS) + ["--with-pkgversion=" + pkgversion, "--extra-cflags=" + cflags]


def configure_line(args, src, build, kroot):
    """The configure command as recorded in the manifest: no directory of the machine that ran it."""
    text = " ".join(["<qemu-src>/configure"] + [str(a) for a in args])
    for real, name in sorted(((str(src), "<qemu-src>"), (str(build), "<qemu-build>"), (str(kroot), "<kayfabe-src>")), key=lambda p: -len(p[0])):
        text = text.replace(real, name)
    return text.replace("-ffile-prefix-map=%s=" % os.path.relpath(src, build), "-ffile-prefix-map=<qemu-src-relative>=")


# ------------------------------------------------------------------------------------------------ version and trust
def read_qemu_version(tree):
    path = Path(tree) / "VERSION"
    try:
        lines = path.read_text().splitlines()
    except OSError:
        raise KitRefused("%s is not a QEMU source tree: no VERSION file" % tree) from None
    if len(lines) != 1 or not re.fullmatch(r"[0-9]+(?:\.[0-9]+)*(?:[-+~.][0-9A-Za-z.]+)?", lines[0].strip()):
        raise KitRefused("%s: VERSION is not one version line: %r" % (path, lines[:3]))
    return lines[0].strip()


def check_supported(version):
    if version not in SUPPORTED_QEMU:
        raise KitRefused("QEMU %s is not a version this kit knows; known: %s" % (version or "(empty)", ", ".join(SUPPORTED_QEMU)))


def read_release_pin(path=RELEASE_PIN):
    signers, signature = [], []
    try:
        lines = Path(path).read_text().splitlines()
    except OSError as e:
        raise KitRefused("cannot read %s: %s" % (path, e.strerror)) from None
    for line in lines:
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(None, 2)
        if parts[0] == "signer" and len(parts) == 3 and re.fullmatch(r"[0-9A-F]{40}", parts[1]):
            signers.append((parts[1], parts[2]))
            continue
        m = re.fullmatch(r"signature (\S+\.sig) sha256=([0-9a-f]{64}) size=([1-9][0-9]*)", line)
        if m:
            signature.append({"name": m.group(1), "sha256": m.group(2), "size": int(m.group(3))})
            continue
        raise KitRefused("%s: not a signer or signature line: %r" % (path, line))
    if not signers or len(signature) != 1:
        raise KitRefused("%s must hold at least one signer and exactly one signature line" % path)
    return {"signers": signers, "signature": signature[0]}


def _gpg(homedir, *args, capture=True):
    if shutil.which("gpg") is None:
        raise KitRefused("gpg is not installed; it is needed to verify the QEMU release signature")
    argv = ["gpg", "--batch", "--no-tty", "--homedir", str(homedir), "--no-autostart", *args]
    return subprocess.run(argv, capture_output=capture, text=True, env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LC_ALL": "C"})


def verify_signature(data, sig, key_file, allowed):
    """Return the primary fingerprint that signed `data`, or raise. A private keyring holds only `key_file`."""
    data, sig, key_file = Path(data), Path(sig), Path(key_file)
    if not sig.is_file() or sig.stat().st_size == 0:
        raise KitRefused("no signature file %s" % sig)
    allowed = set(allowed)
    with tempfile.TemporaryDirectory(prefix="kf-gpg-") as home:
        os.chmod(home, 0o700)
        shown = _gpg(home, "--with-colons", "--show-keys", str(key_file))
        have = set(re.findall(r"^fpr:+([0-9A-F]{40}):", shown.stdout, re.M))
        if shown.returncode != 0 or not have & allowed:
            raise KitRefused("%s holds no key whose fingerprint is a listed QEMU release signer" % key_file)
        if _gpg(home, "--import", str(key_file)).returncode != 0:
            raise KitRefused("cannot import %s" % key_file)
        res = _gpg(home, "--status-fd", "1", "--verify", str(sig), str(data))
    status = res.stdout
    valid = re.findall(r"^\[GNUPG:\] VALIDSIG (\S+)(?: \S+){8}(?: (\S+))?$", status, re.M)
    bad = re.search(r"^\[GNUPG:\] (?:BADSIG|ERRSIG|EXPSIG|EXPKEYSIG|REVKEYSIG|NO_PUBKEY)\b", status, re.M)
    if res.returncode != 0 or bad or len(valid) != 1 or "[GNUPG:] GOODSIG" not in status:
        raise KitRefused("the signature %s does not verify %s" % (sig.name, data.name))
    signing, primary = valid[0][0], (valid[0][1] or valid[0][0])
    if primary not in allowed and signing not in allowed:
        raise KitRefused("%s is signed by %s, which is not a listed QEMU release signer" % (data.name, primary))
    return primary if primary in allowed else signing


# ------------------------------------------------------------------------------------------------ trees and archives
def tree_digest(root):
    """sha256 over (kind, exec bit, content hash, path) of every file and symlink under root, .git excluded; no clock, no owner."""
    root = Path(root)
    lines = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = sorted(d for d in dirnames if d != ".git")
        for name in sorted(filenames + [d for d in dirnames if (Path(dirpath) / d).is_symlink()]):
            if name == ".git":
                continue
            path = Path(dirpath) / name
            rel = path.relative_to(root).as_posix()
            if path.is_symlink():
                lines.append("l - %s %s" % (sha256_bytes(os.readlink(path).encode()), rel))
            else:
                lines.append("f %d %s %s" % (1 if path.stat().st_mode & 0o111 else 0, sha256_file(path), rel))
    return sha256_bytes("\n".join(sorted(lines)).encode() + b"\n")


def _members_from_tree(root):
    root = Path(root)
    out = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames.sort()
        for name in sorted(filenames + [d for d in dirnames if (Path(dirpath) / d).is_symlink()]):
            path = Path(dirpath) / name
            rel = path.relative_to(root).as_posix()
            if path.is_symlink():
                out.append((rel, "link", 0o777, os.readlink(path)))
            else:
                out.append((rel, "file", 0o755 if path.stat().st_mode & 0o111 else 0o644, path))
    return out


def _write_archive(members, out, epoch, prefix, preset):
    """members: (relative name, 'file'|'link', mode, bytes-or-Path-or-linkname). Directories are derived."""
    dirs = {prefix}
    for rel, *_ in members:
        parts = rel.split("/")[:-1]
        for i in range(1, len(parts) + 1):
            dirs.add(prefix + "/" + "/".join(parts[:i]))
    entries = [(d, "dir", 0o755, None) for d in dirs] + [(prefix + "/" + rel, kind, mode, payload) for rel, kind, mode, payload in members]
    entries.sort(key=lambda e: e[0])
    out = Path(out)
    tmp = out.with_name(out.name + ".part")
    with open(tmp, "wb") as raw, lzma.LZMAFile(raw, "wb", format=lzma.FORMAT_XZ, check=lzma.CHECK_CRC64, preset=preset) as xz, \
            tarfile.open(fileobj=xz, mode="w", format=tarfile.GNU_FORMAT) as tf:
        for name, kind, mode, payload in entries:
            info = tarfile.TarInfo(name)
            info.mtime, info.uid, info.gid, info.uname, info.gname, info.mode = epoch, 0, 0, "", "", mode
            if kind == "dir":
                info.type = tarfile.DIRTYPE
                tf.addfile(info)
            elif kind == "link":
                info.type, info.linkname = tarfile.SYMTYPE, payload
                tf.addfile(info)
            elif isinstance(payload, (bytes, bytearray)):
                info.size = len(payload)
                tf.addfile(info, io.BytesIO(payload))
            else:
                info.size = os.path.getsize(payload)
                with open(payload, "rb") as f:
                    tf.addfile(info, f)
    os.replace(tmp, out)


def pack_tree(root, out, epoch, prefix, preset=6):
    _write_archive(_members_from_tree(root), out, epoch, prefix, preset)


def pack_entries(entries, out, epoch, prefix, preset=6):
    _write_archive([(name, "file", 0o644, data) for name, data in entries], out, epoch, prefix, preset)


def safe_extract(archive, dest):
    """Extract a tar archive of unknown origin: no absolute or parent-directory names, no escaping links, no device nodes.

    A symlink with an absolute target is not extracted (upstream QEMU ships one, a dangling /opt/X11/include under roms/edk2);
    the names of the links left out are returned, sorted.
    """
    dest = Path(dest)
    dest.mkdir(parents=True, exist_ok=True)
    dropped = set()
    try:
        with tarfile.open(archive) as tf:
            for m in tf.getmembers():
                name = Path(m.name)
                if name.is_absolute() or ".." in name.parts:
                    raise KitRefused("%s: member %r leaves the extraction directory" % (archive, m.name))
                if not (m.isfile() or m.isdir() or m.issym() or m.islnk()):
                    raise KitRefused("%s: member %r is not a file, directory or link" % (archive, m.name))
                if m.issym() and os.path.isabs(m.linkname):
                    dropped.add(m.name)
                elif m.issym() and os.path.normpath(os.path.join(os.path.dirname(m.name), m.linkname)).startswith(".."):
                    raise KitRefused("%s: link %r points outside the archive" % (archive, m.name))

            def keep(member, path):
                return None if member.name in dropped else tarfile.data_filter(member, path)

            tf.extractall(dest, filter=keep)
    except tarfile.TarError as e:
        raise KitRefused("%s cannot be extracted: %s" % (archive, e)) from None
    return sorted(dropped)


def render_manifest(doc, forbid=()):
    text = json.dumps(doc, sort_keys=True, indent=1, ensure_ascii=True) + "\n"
    for needle in forbid:
        if needle and needle in text:
            raise KitRefused("the manifest would carry the build path %s" % needle)
    return text


# ------------------------------------------------------------------------------------------------ the kayfabe side
def kf3_abi(kroot):
    text = (Path(kroot) / DEVICE_DIR / "kf3.h").read_text()
    m = re.search(r"^#define\s+KF3_ABI\s+(\d+)\b", text, re.M)
    if not m:
        raise KitRefused("%s/kf3.h does not define KF3_ABI" % DEVICE_DIR)
    return int(m.group(1))


def _git(root, *args):
    res = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True,
                         env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_SYSTEM": "/dev/null"})
    if res.returncode != 0:
        raise KitRefused("git %s failed in %s: %s" % (" ".join(args), root, res.stderr.strip()))
    return res.stdout


def kayfabe_identity(root, allow_dirty=False):
    """{revision, source_date_epoch, dirty}: from the git checkout, or from the SOURCE_INFO.json a source tarball carries."""
    root = Path(root)
    if (root / ".git").exists():
        revision = _git(root, "rev-parse", "HEAD").strip()
        epoch = int(_git(root, "log", "-1", "--format=%ct").strip())
        dirty = bool(_git(root, "status", "--porcelain", "--untracked-files=no", "--", *SOURCE_PATHS).strip())
        if dirty and not allow_dirty:
            raise KitRefused("the checkout has uncommitted changes in the shipped paths; commit them or pass --allow-dirty")
        return {"revision": revision, "source_date_epoch": epoch, "dirty": dirty}
    try:
        info = json.loads((root / "SOURCE_INFO.json").read_text())
        ok = (isinstance(info, dict) and re.fullmatch(r"[0-9a-f]{40}", str(info.get("revision", "")))
              and type(info.get("source_date_epoch")) is int and info["source_date_epoch"] > 0)
    except (OSError, ValueError):
        ok = False
    if not ok:
        raise KitRefused("%s is neither a git checkout nor a source tarball tree (no valid SOURCE_INFO.json)" % root)
    return {"revision": info["revision"], "source_date_epoch": info["source_date_epoch"], "dirty": False}


def source_files(root):
    root = Path(root)
    if not (root / ".git").exists():
        raise KitRefused("%s is not a git checkout; the source tarball is made from one" % root)
    out = _git(root, "ls-files", "-z", "--", *SOURCE_PATHS)
    return sorted(set(p for p in out.split("\0") if p))


def asset_names(qemu_version, revision, dirty=False):
    stem = "%s-%s-%s%s" % (ASSET_PREFIX, qemu_version, revision[:8], "-dirty" if dirty else "")
    return {"binary": stem + "-linux-x86_64.tar.xz", "source": stem + "-source.tar.xz", "manifest": stem + "-manifest.json"}


# ------------------------------------------------------------------------------------------------ overlay
def _device_sources(kroot):
    d = Path(kroot) / DEVICE_DIR
    files = sorted(p for p in d.iterdir() if p.is_file() and (p.suffix in (".c", ".h") or p.name == "meson.build")) if d.is_dir() else []
    if not any(p.name == "kf3.c" for p in files) or not any(p.name == "meson.build" for p in files):
        raise KitRefused("%s lacks kf3.c or meson.build" % (d,))
    return files


def apply_overlay(qemu_tree, kroot, archive):
    """Lay the device into `qemu_tree` and return the patch records. Idempotent; refuses before it writes anything."""
    qemu_tree, archive = Path(qemu_tree), Path(archive)
    meson, kconfig = qemu_tree / "hw/misc/meson.build", qemu_tree / "hw/misc/Kconfig"
    if not meson.is_file() or not kconfig.is_file():
        raise KitRefused("%s lacks hw/misc/meson.build or hw/misc/Kconfig: not a QEMU source tree" % qemu_tree)
    if not archive.is_file():
        raise KitRefused("the kayfabe archive %s was not built" % archive)
    sources = _device_sources(kroot)
    dest = qemu_tree / "hw/misc/kf3"
    ours = {p.name for p in sources} | {"libkf_qemu.a"}
    if dest.is_dir() and any(p.name not in ours for p in dest.iterdir()):
        raise KitRefused("%s holds files that are not the kayfabe device: remove them or use a clean tree" % dest)
    dest.mkdir(exist_ok=True)
    records = []
    for p in sources:
        shutil.copyfile(p, dest / p.name)
        records.append({"path": "hw/misc/kf3/" + p.name, "kind": "add-file", "sha256": sha256_file(p)})
    shutil.copyfile(archive, dest / "libkf_qemu.a")
    records.append({"path": "hw/misc/kf3/libkf_qemu.a", "kind": "add-file", "sha256": sha256_file(archive)})
    text = meson.read_text()
    if MESON_HUNK not in text:
        meson.write_text(text + ("" if text.endswith("\n") else "\n") + "\n" + MESON_HUNK + "\n")
    text = kconfig.read_text()
    if not re.search(r"^config KF3$", text, re.M):
        kconfig.write_text(text + ("" if text.endswith("\n") else "\n") + KCONFIG_STANZA)
    records.sort(key=lambda r: r["path"])
    records.append({"path": "hw/misc/Kconfig", "kind": "append-stanza", "sha256": sha256_bytes(KCONFIG_STANZA.encode())})
    records.append({"path": "hw/misc/meson.build", "kind": "append-line", "text": MESON_HUNK})
    return records


# ------------------------------------------------------------------------------------------------ environment and runner
def default_homes(caller):
    home = caller.get("HOME", "")
    return caller.get("CARGO_HOME") or os.path.join(home, ".cargo"), caller.get("RUSTUP_HOME") or os.path.join(home, ".rustup")


def build_env(*, epoch, kroot, cargo_home, rustup_home, target_dir, caller):
    """The whole environment of every build step: a fixed set, plus the few variables that select tools. Nothing else passes."""
    env = {k: caller[k] for k in PASS_ENV if k in caller}
    remaps = [(str(kroot), MAP_KAYFABE), (str(cargo_home), MAP_CARGO), (str(rustup_home), MAP_RUSTUP), (str(target_dir), MAP_TARGET)]
    env.update({
        "SOURCE_DATE_EPOCH": str(epoch), "TZ": "UTC", "LC_ALL": "C", "LANG": "C", "PYTHONHASHSEED": "0", "CARGO_INCREMENTAL": "0",
        "CARGO_TARGET_DIR": str(target_dir),
        "RUSTFLAGS": " ".join("--remap-path-prefix=%s=%s" % pair for pair in remaps),
    })
    return env


def run_logged(argv, cwd, env, log):
    with open(log, "ab") as f:
        f.write(("\n$ " + " ".join(map(str, argv)) + "\n").encode())
        f.flush()
        try:
            return subprocess.run([str(a) for a in argv], cwd=cwd, env=env, stdout=f, stderr=subprocess.STDOUT).returncode
        except FileNotFoundError:
            f.write(("command not found: %s\n" % argv[0]).encode())
            return 127


def probe_tool_versions(env):
    def first(argv):
        try:
            out = subprocess.run(argv, env=env, capture_output=True, text=True).stdout.strip().splitlines()
            return out[0].strip() if out else "unavailable"
        except OSError:
            return "missing"
    found = {name: first([name, "--version"]) for name in ("cc", "ld", "rustc", "cargo", "ninja")}
    found["python3"] = first(["python3", "--version"])
    for lib in LIBS_PROBED:
        found["pkg-config:" + lib] = first(["pkg-config", "--modversion", lib])
    return dict(sorted(found.items()))


# ------------------------------------------------------------------------------------------------ the QEMU input
@dataclasses.dataclass(frozen=True)
class QemuInput:
    kind: str  # official | tarball | tree
    path: Path = None
    sig: Path = None
    require_signature: bool = False

    @classmethod
    def official(cls):
        return cls("official")

    @classmethod
    def tarball(cls, path, sig=None, require_signature=False):
        return cls("tarball", Path(path), Path(sig) if sig else None, require_signature)

    @classmethod
    def tree(cls, path):
        return cls("tree", Path(path))


def _fetch_official(dest):
    """The pinned tarball and its signature, through kf_trust.py; the tarball's pin is scripts/bench/qemu.pin."""
    sys.path.insert(0, str(ROOT / "scripts" / "ci"))
    import kf_trust
    import qemu_source
    try:
        tarball = qemu_source.fetch(dest)
        pin = read_release_pin()["signature"]
        sig = Path(dest) / pin["name"]
        kf_trust.resolve("qemu-source-signature", sig, mode=kf_trust.MODE_GIT_ONLY)
    except (qemu_source.Refused, kf_trust.TrustError) as e:
        raise KitRefused("cannot obtain the pinned QEMU release: %s" % e) from None
    return Path(tarball), sig


def resolve_qemu(qi, work):
    """Put an unmodified copy of the QEMU source at work/src; return (version, manifest 'source' record)."""
    src = work / "src"
    if qi.kind == "tree":
        version = read_qemu_version(qi.path)
        check_supported(version)
        source = {"kind": "tree", "tree_sha256": tree_digest(qi.path), "anchors": ["user"]}
        shutil.copytree(qi.path, src, symlinks=True, ignore=shutil.ignore_patterns(".git"))
        return version, source
    pin = read_release_pin()
    if qi.kind == "official":
        tarball, sig = _fetch_official(work / "download")
        anchors = ["qemu-release-signature", "upstream-pin"]
    else:
        tarball, sig = qi.path, qi.sig or Path(str(qi.path) + ".sig")
        anchors = ["user"]
    signer = None
    if qi.kind == "official" or qi.require_signature or sig.is_file():
        signer = verify_signature(tarball, sig, RELEASE_KEYS, [f for f, _ in pin["signers"]])
        if qi.kind == "tarball":
            anchors = ["qemu-release-signature"]
    unpack = work / "unpack"
    dropped = safe_extract(tarball, unpack)
    tops = [p for p in unpack.iterdir()]
    if len(tops) != 1 or not tops[0].is_dir():
        raise KitRefused("%s must hold exactly one top-level directory" % tarball)
    version = read_qemu_version(tops[0])
    check_supported(version)
    tops[0].rename(src)
    source = {"kind": "tarball", "name": Path(tarball).name, "sha256": sha256_file(tarball), "signer": signer, "anchors": sorted(anchors),
              "dropped_absolute_links": dropped}
    return version, source


# ------------------------------------------------------------------------------------------------ build
def _prepare_workdir(workdir):
    workdir = Path(workdir).absolute()
    if workdir.exists():
        if not (workdir / WORK_MARK).is_file():
            raise KitRefused("%s exists and is not a kit work directory; remove it or choose another" % workdir)
        shutil.rmtree(workdir)
    workdir.mkdir(parents=True)
    (workdir / WORK_MARK).write_text("kayfabe qemu kit work directory\n")
    return workdir


def _step(runner, argv, cwd, env, log, what):
    rc = runner([str(a) for a in argv], cwd, env, log)
    if rc != 0:
        tail = ""
        with contextlib.suppress(OSError):
            tail = "\n".join(Path(log).read_text(errors="replace").splitlines()[-30:])
        raise BuildFailed("%s failed (exit %s); log %s\n%s" % (what, rc, log, tail))


README = """kayfabe QEMU %(qemu)s (kayfabe revision %(rev)s)

This is QEMU %(qemu)s with the kayfabe `kf3-gpu` device linked in. QEMU is licensed under the GNU GPL version 2 (COPYING); the
kayfabe parts under Apache-2.0 OR GPL-2.0-or-later (LICENSE). The complete corresponding source of this exact build is the
`-source.tar.xz` published next to this file; `scripts/release/build_qemu.py build --source-tarball <it>` rebuilds it.
Use: bin/qemu-system-x86_64 -device kf3-gpu,gpu-minor=0 ... (docs/howto/installing.md). Nothing here is installed by the kit.
"""


def build(*, kayfabe_root, qemu, out, workdir, jobs, runner=run_logged, tool_versions=probe_tool_versions, caller_env=None,
          allow_dirty=False, source_tarball=None):
    """Build the binary tarball and the manifest into `out`. Returns {'out', 'assets', 'manifest'}."""
    kroot, out = Path(kayfabe_root).absolute(), Path(out).absolute()
    caller_env = dict(os.environ if caller_env is None else caller_env)
    ident = kayfabe_identity(kroot, allow_dirty)
    work = _prepare_workdir(workdir)
    version, qsource = resolve_qemu(qemu, work)
    names = asset_names(version, ident["revision"], ident["dirty"])
    if out.exists() and any((out / n).exists() for n in names.values()):
        raise KitRefused("%s already holds this build's assets; use an empty directory" % out)
    srcdir, builddir, target, stage, logs = work / "src", work / "build", work / "target", work / "stage", work / "logs"
    for d in (builddir, logs):
        d.mkdir()
    cargo_home, rustup_home = default_homes(caller_env)
    env = build_env(epoch=ident["source_date_epoch"], kroot=kroot, cargo_home=cargo_home, rustup_home=rustup_home, target_dir=target,
                    caller=caller_env)
    vendored, cargo_config = (kroot / "vendor").is_dir() and (kroot / ".cargo" / "config.toml").is_file(), []
    if not vendored:
        _step(runner, ["cargo", "vendor", "--locked", "--manifest-path", kroot / "Cargo.toml", work / "vendor"], kroot, env, logs / "vendor.log",
              "cargo vendor")
        cargo_config = ["--config", 'source.crates-io.replace-with="vendored-sources"', "--config",
                        'source.vendored-sources.directory="%s"' % (work / "vendor")]
        env["RUSTFLAGS"] += " --remap-path-prefix=%s=%s/vendor" % (work / "vendor", MAP_KAYFABE)
    _step(runner, ["cargo", *CARGO_ARGS, *cargo_config, "-j", str(jobs)], kroot, env, logs / "cargo.log", "cargo build of kf-qemu")
    archive = target / "release" / "libkf_qemu.a"
    patches = apply_overlay(srcdir, kroot, archive)
    pkgversion = "kayfabe-" + ident["revision"][:8] + ("-dirty" if ident["dirty"] else "")
    cargs = configure_args(src=srcdir, build=builddir, kroot=kroot, pkgversion=pkgversion)
    _step(runner, [srcdir / "configure", *cargs], builddir, env, logs / "configure.log", "QEMU configure")
    _step(runner, ["ninja", "-C", builddir, "-j", str(jobs)], builddir, env, logs / "ninja.log", "QEMU build")
    install_env = {**env, "DESTDIR": str(stage)}
    _step(runner, ["ninja", "-C", builddir, "install"], builddir, install_env, logs / "install.log", "QEMU install")
    prefix_dir = stage / "usr" / "local"
    binary = prefix_dir / "bin" / PACKAGE_NAME
    if not binary.is_file():
        raise BuildFailed("the install produced no %s" % binary)

    top = names["binary"][: -len("-linux-x86_64.tar.xz")]
    pkg = work / "pkg"
    shutil.copytree(prefix_dir, pkg, symlinks=True)
    for doc in DOC_FILES:
        if (kroot / doc).is_file():
            shutil.copyfile(kroot / doc, pkg / doc)
    if (srcdir / "COPYING").is_file():
        shutil.copyfile(srcdir / "COPYING", pkg / "COPYING")
    (pkg / "README.txt").write_text(README % {"qemu": version, "rev": ident["revision"][:8]})
    staged = work / "assets"
    staged.mkdir()
    pack_tree(pkg, staged / names["binary"], ident["source_date_epoch"], top)

    outputs = [{"name": "bin/" + PACKAGE_NAME, "sha256": sha256_file(binary), "size": binary.stat().st_size},
               {"name": names["binary"], "sha256": sha256_file(staged / names["binary"]), "size": (staged / names["binary"]).stat().st_size}]
    manifest = {
        "schema": SCHEMA,
        "asset_prefix": ASSET_PREFIX,
        "qemu": {"version": version, "source": qsource},
        "kayfabe": {"revision": ident["revision"], "dirty": ident["dirty"], "kf3_abi": kf3_abi(kroot), "source_date_epoch": ident["source_date_epoch"]},
        "source_tarball": None if source_tarball is None else dict(source_tarball),
        "cargo": list(CARGO_ARGS),
        "patches": patches,
        "configure": {"args": [configure_line([a], srcdir, builddir, kroot) for a in cargs], "line": configure_line(cargs, srcdir, builddir, kroot)},
        "environment": {k: env[k] for k in ("SOURCE_DATE_EPOCH", "TZ", "LC_ALL", "LANG", "CARGO_INCREMENTAL")}
        | {"path_maps": sorted([MAP_SRC, MAP_BUILD, MAP_KAYFABE, MAP_CARGO, MAP_RUSTUP, MAP_TARGET])},
        "toolchain": tool_versions(env),
        "outputs": sorted(outputs, key=lambda o: o["name"]),
    }
    text = render_manifest(manifest, forbid=[str(work), str(kroot), str(out), str(cargo_home), str(rustup_home)])
    (staged / names["manifest"]).write_text(text)
    out.mkdir(parents=True, exist_ok=True)
    for key in ("binary", "manifest"):
        os.replace(staged / names[key], out / names[key])
    return {"out": str(out), "assets": names, "manifest": manifest}


# ------------------------------------------------------------------------------------------------ the source tarball
BUILDING = """Corresponding source of %(name)s (QEMU %(qemu)s + kayfabe revision %(rev)s).

  kayfabe/   the kayfabe source (the device qemu/hw/misc/kf3/, the Rust crates, the build kit scripts/release/), the vendored
             crates.io sources (vendor/) and the licence texts
  qemu/      the unmodified upstream QEMU release tarball and its detached signature

To rebuild the binary (Linux; the packages of kayfabe/docs/howto/building.md step 2 and gnupg; the Rust toolchain named in
kayfabe/rust-toolchain.toml): run, from the directory that holds this source tarball's .tar.xz file,

  python3 -I <this directory>/kayfabe/scripts/release/build_qemu.py build --source-tarball <the .tar.xz file> --out out

The binary tarball is byte-identical to the published one when the toolchain is the one its manifest records.
"""


def make_source_tarball(*, kayfabe_root, tarball, sig, out, runner=run_logged, caller_env=None, workdir=None, allow_dirty=False):
    kroot, out = Path(kayfabe_root).absolute(), Path(out).absolute()
    caller_env = dict(os.environ if caller_env is None else caller_env)
    ident = kayfabe_identity(kroot, allow_dirty)
    pin = read_release_pin()
    signer = verify_signature(tarball, sig, RELEASE_KEYS, [f for f, _ in pin["signers"]])
    m = re.fullmatch(r"qemu-(.+)\.tar\.xz", Path(tarball).name)
    if not m:
        raise KitRefused("%s is not a qemu-<version>.tar.xz release tarball" % tarball)
    check_supported(m.group(1))
    names = asset_names(m.group(1), ident["revision"], ident["dirty"])
    work = _prepare_workdir(workdir or out.with_name(out.name + ".source-work"))
    staging = work / "kayfabe"
    for rel in source_files(kroot):
        dest = staging / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(kroot / rel, dest, follow_symlinks=False)
    vendor_env = {k: caller_env[k] for k in PASS_ENV if k in caller_env}
    (work / "logs").mkdir()
    _step(runner, ["cargo", "vendor", "--locked", "--manifest-path", staging / "Cargo.toml", staging / "vendor"], staging, vendor_env,
          work / "logs" / "vendor.log", "cargo vendor")
    cargo_config = staging / ".cargo" / "config.toml"
    cargo_config.parent.mkdir()
    cargo_config.write_text('[source.crates-io]\nreplace-with = "vendored-sources"\n\n[source.vendored-sources]\ndirectory = "vendor"\n')
    (staging / "SOURCE_INFO.json").write_text(render_manifest({"revision": ident["revision"], "source_date_epoch": ident["source_date_epoch"]}))
    top = names["source"][: -len(".tar.xz")]
    members = []
    for q in sorted(staging.rglob("*")):
        rel = "kayfabe/" + q.relative_to(staging).as_posix()
        if q.is_symlink():
            members.append((rel, "link", 0o777, os.readlink(q)))
        elif q.is_file():
            members.append((rel, "file", 0o755 if q.stat().st_mode & 0o111 else 0o644, q))
    members.append(("qemu/" + Path(tarball).name, "file", 0o644, Path(tarball)))
    members.append(("qemu/" + Path(sig).name, "file", 0o644, Path(sig)))
    members.append(("BUILDING.txt", "file", 0o644, (BUILDING % {"name": top, "qemu": m.group(1), "rev": ident["revision"][:8]}).encode()))
    out.mkdir(parents=True, exist_ok=True)
    _write_archive(members, out / names["source"], ident["source_date_epoch"], top, 1)
    shutil.rmtree(work)
    return {"out": str(out), "name": names["source"], "sha256": sha256_file(out / names["source"]), "signer": signer}


def open_source_tarball(path, work):
    """Extract a source tarball into `work`; return (kayfabe root, QemuInput, the manifest record of the tarball itself)."""
    path = Path(path)
    if not path.is_file():
        raise KitRefused("no source tarball %s" % path)
    unpack = _prepare_workdir(work) / "source-tarball"
    safe_extract(path, unpack)
    tops = list(unpack.iterdir())
    if len(tops) != 1 or not (tops[0] / "kayfabe").is_dir():
        raise KitRefused("%s is not a kayfabe source tarball" % path)
    tarballs = sorted((tops[0] / "qemu").glob("qemu-*.tar.xz"))
    if len(tarballs) != 1:
        raise KitRefused("%s must carry exactly one QEMU release tarball" % path)
    qi = QemuInput.tarball(tarballs[0], Path(str(tarballs[0]) + ".sig"), require_signature=True)
    return tops[0] / "kayfabe", qi, {"name": path.name, "sha256": sha256_file(path)}


# ------------------------------------------------------------------------------------------------ comparing builds
def compare_outputs(a, b, ignore_keys=()):
    """Differences between two output directories; manifests compare as data with `ignore_keys` removed."""
    a, b = Path(a), Path(b)
    names_a, names_b = sorted(p.name for p in a.iterdir()), sorted(p.name for p in b.iterdir())
    diff = ["only in %s: %s" % (side, n) for side, mine, other in (("a", names_a, names_b), ("b", names_b, names_a)) for n in mine if n not in other]
    for name in sorted(set(names_a) & set(names_b)):
        x, y = (a / name).read_bytes(), (b / name).read_bytes()
        if x == y:
            continue
        if name.endswith("manifest.json"):
            dx, dy = json.loads(x), json.loads(y)
            for key in ignore_keys:
                for doc in (dx, dy):
                    node = doc
                    *parents, leaf = key.split(".")
                    for part in parents:
                        node = node.get(part) if isinstance(node, dict) else None
                    if isinstance(node, dict):
                        node.pop(leaf, None)
            if dx == dy:
                continue
        diff.append("%s differs (sha256 %s vs %s)" % (name, sha256_bytes(x)[:16], sha256_bytes(y)[:16]))
    return diff


# ------------------------------------------------------------------------------------------------ command line
def _add_inputs(p):
    g = p.add_argument_group("QEMU input (default: the pinned official release)")
    g.add_argument("--qemu-tarball", metavar="FILE", help="a QEMU release tarball you have (checked against FILE.sig when that exists)")
    g.add_argument("--qemu-tree", metavar="DIR", help="a QEMU source tree you have; never modified, its .git is ignored")
    g.add_argument("--source-tarball", metavar="FILE", help="a source tarball made by `source`: kayfabe, vendored crates and the signed QEMU tarball")
    p.add_argument("--kayfabe-root", metavar="DIR", default=str(ROOT), help="the kayfabe checkout (default: the one this script is in)")
    p.add_argument("--allow-dirty", action="store_true", help="build a checkout with uncommitted changes; the result is named -dirty")


def _inputs(p, a):
    given = [x for x in (a.qemu_tarball, a.qemu_tree, a.source_tarball) if x]
    if len(given) > 1:
        p.error("give at most one of --qemu-tarball, --qemu-tree and --source-tarball")
    if a.source_tarball and a.kayfabe_root != str(ROOT):
        p.error("--source-tarball carries its own kayfabe tree; do not combine it with --kayfabe-root")
    if a.qemu_tree:
        return QemuInput.tree(a.qemu_tree)
    if a.qemu_tarball:
        return QemuInput.tarball(a.qemu_tarball)
    return QemuInput.official()


def _plan(a, qi):
    kroot = Path(a.kayfabe_root)
    version = read_qemu_version(qi.path) if qi.kind == "tree" else (qi.path.name[5:-7] if qi.kind == "tarball" else SUPPORTED_QEMU[0])
    check_supported(version)
    source = ({"kind": "tree", "tree_sha256": tree_digest(qi.path)} if qi.kind == "tree" else
              {"kind": "tarball", "name": "qemu-%s.tar.xz" % version, "note": "not fetched by plan"})
    where = ("/work/src", "/work/build", "/work/kayfabe")
    cargs = configure_args(src=where[0], build=where[1], kroot=where[2], pkgversion="kayfabe-<revision>")
    plan = {"qemu_version": version, "qemu_source": source, "supported_qemu": list(SUPPORTED_QEMU),
            "patches": [{"path": "hw/misc/kf3/" + p.name, "kind": "add-file", "sha256": sha256_file(p)} for p in _device_sources(kroot)]
            + [{"path": "hw/misc/kf3/libkf_qemu.a", "kind": "add-file", "note": "cargo build --release --locked -p kf-qemu"},
               {"path": "hw/misc/Kconfig", "kind": "append-stanza"}, {"path": "hw/misc/meson.build", "kind": "append-line", "text": MESON_HUNK}],
            "configure_line": configure_line(cargs, *where)}
    return plan


def _build_one(a, qi, out, workdir, source_tarball=None):
    kroot, qemu, record = Path(a.kayfabe_root), qi, source_tarball
    if a.source_tarball:
        kroot, qemu, record = open_source_tarball(a.source_tarball, Path(workdir).with_name(Path(workdir).name + "-src"))
    return build(kayfabe_root=kroot, qemu=qemu, out=out, workdir=workdir, jobs=a.jobs, allow_dirty=a.allow_dirty, source_tarball=record)


def main(argv=None):
    p = argparse.ArgumentParser(prog="build_qemu.py", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    for name in ("plan", "build", "source", "reproduce"):
        s = sub.add_parser(name, help=name, formatter_class=argparse.RawDescriptionHelpFormatter)
        _add_inputs(s)
        if name != "plan":
            s.add_argument("--out", required=True, metavar="DIR", help="output directory")
            s.add_argument("--workdir", metavar="DIR", help="scratch directory, recreated each run (default: OUT.work)")
        if name in ("build", "reproduce"):
            s.add_argument("--jobs", type=int, default=os.cpu_count() or 2, help="parallel jobs")
    a = p.parse_args(argv)
    qi = _inputs(p, a)
    try:
        if a.cmd == "plan":
            print(json.dumps(_plan(a, qi), indent=1, sort_keys=True))
            return 0
        out = Path(a.out).absolute()
        workdir = Path(a.workdir).absolute() if a.workdir else out.with_name(out.name + ".work")
        if a.cmd == "build":
            r = _build_one(a, qi, out, workdir)
            print("KIT_BUILT %s" % " ".join(str(out / n) for n in (r["assets"]["binary"], r["assets"]["manifest"])))
            return 0
        if a.cmd == "source":
            if qi.kind == "tree" or a.source_tarball:
                p.error("the source tarball needs a QEMU release tarball: use the default or --qemu-tarball")
            work = _prepare_workdir(workdir)
            if qi.kind == "official":
                tarball, sig = _fetch_official(work / "download")
            else:
                tarball, sig = qi.path, Path(str(qi.path) + ".sig")
            r = make_source_tarball(kayfabe_root=a.kayfabe_root, tarball=tarball, sig=sig, out=out, workdir=work.with_name(work.name + "-tree"),
                                    allow_dirty=a.allow_dirty)
            shutil.rmtree(work)
            print("KIT_SOURCE %s sha256=%s" % (Path(r["out"]) / r["name"], r["sha256"]))
            return 0
        first, second = out / "a", out / "b"
        _build_one(a, qi, first, workdir / "a")
        if a.source_tarball:
            second_args = argparse.Namespace(**{**vars(a), "source_tarball": None})
            _build_one(second_args, QemuInput.official(), second, workdir / "b")
            ignore = ("source_tarball", "qemu.source.anchors")
        else:
            _build_one(a, qi, second, workdir / "b")
            ignore = ()
        diff = compare_outputs(first, second, ignore)
        for line in diff:
            print("KIT_NOT_REPRODUCIBLE " + line, file=sys.stderr)
        if diff:
            return 3
        print("KIT_REPRODUCIBLE %s" % ", ".join(sorted(n for n in os.listdir(first))))
        return 0
    except KitRefused as e:
        print("KIT_REFUSED: %s" % e, file=sys.stderr)
        return 1
    except BuildFailed as e:
        print("KIT_BUILD_FAILED: %s" % e, file=sys.stderr)
        return 3


if __name__ == "__main__":
    sys.exit(main())
