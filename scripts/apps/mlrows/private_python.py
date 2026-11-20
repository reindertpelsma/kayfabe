#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""A private CPython 3.12 for the HOST side of the open ML rows (stdlib only; GPU-free tested in scripts/ci/test_mlrows.py).

STATUS: LIVE, 2026-11-20 (docs/design/V3_OPEN_ML_ROWS.md section 3a). Code and GPU-free tests; nothing here has run on a box. NOT DONE: the sha256 of the source tarball is
not in scripts/ci/trust_manifest.json yet (it needs one measurement on a machine with network), so `ensure` refuses by name (`python_source_refused:TRUST_UNKNOWN_ARTIFACT`)
until `pin-entry` has been run and its entry committed. The refusal is the safe state: the ML rows then say `python_too_old` as before.

Why: the locks of the ML rows are cp312 and the stock Ubuntu 22.04 host of a Vast box has Python 3.10, so `mlrows.py setup` refused `python_too_old: 3.10 < 3.12` (MEASURED
2026-11-20). Owner ruling 2026-11-20: build CPython 3.12 from the python.org SOURCE tarball on the provisioning host, into a prefix of its own (never the system Python), anchored in git
through scripts/ci/kf_trust.py (the manifest artifact `ARTIFACT` below: an upstream pin of the tarball's sha256, the rule of docs/design/V3_STANDALONE_TRUST.md), with the build
dependencies from the OS archive (profile `pybuild` of scripts/bench/provision_box.sh). No PPA, no third-party binary.

    private_python.py ensure --root MLROWS_ROOT [--if-needed] [--jobs N]     build once (a marker says so), print one PRIVATE_PYTHON line; exit 0 unless the build itself failed
    private_python.py pin-entry --tarball FILE [--md5 HEX] [--measured DATE]  print the trust-manifest entry for a tarball fetched by hand (run it where there is network)

The prefix is <root>/python312 (root = the mlrows root, the one `mlrows.py setup --root` gets), so `mlrows.py` finds it from its own root and `push` ships it with the venvs, whose
interpreter links point into it. Static libpython (no --enable-shared): the interpreter needs no LD_LIBRARY_PATH. The tarball is fetched ONLY through kf_trust; this file has no
download of its own, and an artifact the manifest does not list is a named refusal, never a fallback.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tarfile
import time

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
KF_TRUST = REPO / "scripts" / "ci" / "kf_trust.py"
PROVISION = REPO / "scripts" / "bench" / "provision_box.sh"

PY_VERSION = "3.12.10"                                  # the 3.12 the repository already pins for Windows (docs/design/V3_WINDOWS_APP_MATRIX.md); one constant
SERIES = ".".join(PY_VERSION.split(".")[:2])
ARTIFACT = "python-%s-source" % PY_VERSION              # the trust manifest's name for the tarball: its origin and anchor live there, not here
PRIVATE_DIR = "python" + SERIES.replace(".", "")        # <mlrows root>/python312
PACKAGE_PROFILE = "pybuild"                             # scripts/bench/provision_box.sh --packages pybuild: the one list of the build dependencies
MARKER = ".kf_private_python.json"
MARKER_SCHEMA = "kayfabe.private-python/1"
REQUIRED_MODULES = ("ssl", "zlib", "ctypes", "bz2", "lzma", "sqlite3", "venv", "ensurepip")   # a missing header builds a Python without the module and says nothing
TIMEOUT_S = {"fetch": 900, "apt": 900, "configure": 600, "make": 2400, "install": 600, "dpkg": 30, "probe": 60}      # their sum (5400 s) is below the stage cap of the step (6000 s)
ENV_KEEP = ("PATH", "TZ", "TMPDIR")
APT_SHIM_ENV = ("BASH_ENV", "KF_PAYLOAD_ROOT", "KF_APT_CACHE_ROOT")      # what makes `apt-get` the payload's cache-only shim (scripts/ci/vast_payload_hooks.sh): only the apt call gets them
PROBE_RX = re.compile(r"^KF_PY version=(\d+)\.(\d+)\.(\d+) include=(.*) missing=(\S+)$", re.M)
PROBE_CODE = ("import importlib, sys, sysconfig\n"
              "bad = []\n"
              "for name in %r:\n"
              "    try:\n"
              "        importlib.import_module(name)\n"
              "    except Exception:\n"
              "        bad.append(name)\n"
              "print('KF_PY version=%%d.%%d.%%d include=%%s missing=%%s' %% (tuple(sys.version_info[:3]) + (sysconfig.get_paths()['include'], ','.join(bad) or '-')))\n") % (REQUIRED_MODULES,)
ORIGIN = "https://www.python.org/ftp/python/%s/Python-%s.tgz"                      # the manifest entry's origin (pin-entry prints it); never fetched from here
RELEASE_PAGE = "https://www.python.org/downloads/release/python-%s/"               # where anyone re-checks the pin: it lists the MD5, the size, the GPG and Sigstore files
PACKAGE_RX = re.compile(r"[a-z0-9][a-z0-9+.-]*\Z")
TRUST_CODE_RX = re.compile(r"TRUST_[A-Z_]+")


def kv(**fields):
    return " ".join("%s=%s" % (k, re.sub(r"\s+", "_", str(v)) if v is not None else "-") for k, v in fields.items())


def run_cmd(argv, env=None, timeout=60, cwd=None):
    """-> (rc, output text, timed_out). The whole process group is killed on a timeout. Replaced by the tests."""
    try:
        p = subprocess.Popen(argv, env=env, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True)
    except OSError as exc:
        return 127, str(exc), False
    try:
        out, _ = p.communicate(timeout=timeout)
        return p.returncode, out.decode("utf-8", "replace"), False
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        out, _ = p.communicate()
        return 124, out.decode("utf-8", "replace"), True


# ------------------------------------------------------------------------------------------------ where things are, and the recipe
def prefix_of(root):
    return Path(os.path.abspath(root)) / PRIVATE_DIR


def interpreter_of(root):
    return prefix_of(root) / "bin" / ("python" + SERIES)


def host_version():
    return tuple(sys.version_info[:3])


def default_jobs():
    return max(1, min(8, os.cpu_count() or 2))


def recipe(prefix, src, jobs):
    """The build as data: [(step, argv, cwd)]. No PGO or LTO (a Vast box has a deadline, and the rows measure the driver, not the interpreter), no --enable-shared,
    `altinstall` (the interpreter is `bin/python3.12` only), no pip in the prefix (every venv gets its own from the bundled wheels)."""
    return [("configure", ["./configure", "--prefix=" + str(prefix), "--with-ensurepip=no"], src),
            ("make", ["make", "-j%d" % jobs], src),
            ("install", ["make", "altinstall"], src)]


def recipe_id(prefix):
    """What a marker must agree with: a changed version, prefix, configure line or module list is a rebuild (jobs and the source directory are not part of it)."""
    steps = [[name, [a for a in argv if not a.startswith("-j")]] for name, argv, _ in recipe(prefix, "SRC", 1)]
    return hashlib.sha256(json.dumps([PY_VERSION, ARTIFACT, str(prefix), steps, list(REQUIRED_MODULES)], sort_keys=True).encode()).hexdigest()


def build_env(environ=None, trust=False, apt=False):
    """An allowlist of the caller's environment (PATH, TZ, TMPDIR), plus the KF_TRUST_* variables for the one call that goes through kf_trust (its mode, evidence directory and
    lift) and the variables that make apt-get the payload's cache-only shim for the one apt call."""
    source = os.environ if environ is None else environ
    env = {k: v for k, v in source.items() if k in ENV_KEEP or (trust and k.startswith("KF_TRUST_")) or (apt and k in APT_SHIM_ENV)}
    if apt:
        env["DEBIAN_FRONTEND"] = "noninteractive"
    env.setdefault("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
    env.update({"LANG": "C", "LC_ALL": "C"})
    return env


# ------------------------------------------------------------------------------------------------ is there a usable one
def probe(exe, run=None):
    """Run `exe` and ask it who it is. -> {"version": (3, 12, 10), "include": dir, "missing": [modules]} or None when it does not run or does not answer."""
    run = run or run_cmd
    if not Path(exe).exists():
        return None
    rc, out, _ = run([str(exe), "-I", "-c", PROBE_CODE], timeout=TIMEOUT_S["probe"])
    m = PROBE_RX.search(out or "") if rc == 0 else None
    if not m:
        return None
    return {"version": tuple(int(x) for x in m.group(1, 2, 3)), "include": m.group(4), "missing": [] if m.group(5) == "-" else m.group(5).split(",")}


def read_marker(root):
    try:
        doc = json.loads((prefix_of(root) / MARKER).read_text())
    except (OSError, ValueError):
        return None
    return doc if isinstance(doc, dict) and doc.get("schema") == MARKER_SCHEMA else None


def usable(root, run=None):
    """The probe of the private interpreter when it is complete (a marker of THIS recipe, a Python that runs, every required module), else None.
    A prefix without a marker is a half-made build and is never used."""
    run = run or run_cmd
    marker = read_marker(root)
    if not marker or marker.get("version") != PY_VERSION or marker.get("recipe") != recipe_id(prefix_of(root)):
        return None
    got = probe(interpreter_of(root), run)
    if not got or got["missing"] or ".".join(map(str, got["version"])) != PY_VERSION:
        return None
    return got


# ------------------------------------------------------------------------------------------------ the build
def package_policy(run):
    """The build dependencies, from the one list there is (provision_box.sh --packages pybuild). -> (names, problem)."""
    rc, out, _ = run(["bash", str(PROVISION), "--packages", PACKAGE_PROFILE], timeout=TIMEOUT_S["dpkg"])
    names = out.split() if rc == 0 else []
    if not names or not all(PACKAGE_RX.match(n) for n in names):
        return [], "package_policy_unreadable"
    return names, None


def missing_packages(run, names):
    out = []
    for name in names:
        rc, text, _ = run(["dpkg-query", "-W", "-f=${Status}", name], timeout=TIMEOUT_S["dpkg"])
        if rc != 0 or "install ok installed" not in text:
            out.append(name)
    return out


def install_packages(run, names, env):
    """Through bash so that the payload's cache-only apt-get shim (a shell function, BASH_ENV) is the one that runs inside a payload; the plain apt-get anywhere else."""
    return run(["bash", "-c", 'apt-get install -y -qq --no-install-recommends "$@"', "kf-pybuild"] + list(names), env=env, timeout=TIMEOUT_S["apt"])


def fetch_source(run, out, env):
    """The tarball, through kf_trust and nothing else. -> (ok, reason). An artifact the manifest does not list, a pin that disagrees, a transfer that fails: all named."""
    rc, text, timed_out = run([sys.executable, "-I", str(KF_TRUST), "fetch", ARTIFACT, "--out", str(out)], env=env, timeout=TIMEOUT_S["fetch"])
    if rc == 0 and Path(out).is_file():
        return True, ""
    code = TRUST_CODE_RX.search(text or "")
    return False, "python_source_refused:" + (code.group(0) if code else ("timeout" if timed_out else "rc%d" % rc))


def safe_members(tar, dest):
    """The members of a source tarball that may be extracted below `dest`: regular files and directories, and symbolic links that stay inside. Anything else refuses the tarball."""
    root = os.path.realpath(dest)
    for m in tar.getmembers():
        target = os.path.realpath(os.path.join(root, m.name))
        if os.path.isabs(m.name) or ".." in Path(m.name).parts or not (target == root or target.startswith(root + os.sep)):
            raise ValueError("member_outside_the_tree:" + m.name[:60])
        if m.issym() or m.islnk():
            link = os.path.realpath(os.path.join(os.path.dirname(target), m.linkname)) if m.issym() else os.path.realpath(os.path.join(root, m.linkname))
            if os.path.isabs(m.linkname) or not (link == root or link.startswith(root + os.sep)):
                raise ValueError("link_outside_the_tree:" + m.name[:60])
        elif not (m.isreg() or m.isdir()):
            raise ValueError("special_member:" + m.name[:60])
        yield m


def extract_source(tgz, dest):
    """Extract into an empty `dest`; the tarball must hold exactly one top directory, Python-<version>. -> its path. ValueError names the refusal."""
    dest = Path(dest)
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    try:
        with tarfile.open(tgz, "r:gz") as tar:
            members = list(safe_members(tar, dest))
            tops = {Path(m.name).parts[0] for m in members if Path(m.name).parts}
            if tops != {"Python-" + PY_VERSION}:
                raise ValueError("unexpected_top_directory:" + ",".join(sorted(tops))[:60])
            tar.extractall(dest, members=members, **({"filter": "data"} if hasattr(tarfile, "data_filter") else {}))
    except (tarfile.TarError, OSError, EOFError) as exc:
        raise ValueError("unreadable_tarball:" + type(exc).__name__) from None
    return dest / ("Python-" + PY_VERSION)


def tail(text):
    lines = [l.strip() for l in (text or "").splitlines() if l.strip()]
    return re.sub(r"[^A-Za-z0-9_.:=/-]+", "_", lines[-1])[:80] if lines else "no_output"


def result(status, reason, root=None, built=False, version=None):
    return {"status": status, "reason": reason, "built": built, "version": version, "python": str(interpreter_of(root)) if root and version else None}


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def ensure(root, run=None, if_needed=False, jobs=None, running=None, environ=None, clock=time.time, root_user=None):
    """Make sure `<root>/python312` holds a complete private Python. Idempotent (a marker of this recipe and a probe), never touches the system Python, never raises for a refusal.
    -> {status ok|NOT_RUN|FAIL, reason, built, version, python}. NOT_RUN is a statement about the machine or the repository's configuration (a missing build dependency, an artifact
    the trust manifest does not anchor, a transfer that failed); FAIL is a build that ran and did not produce a complete interpreter."""
    run = run or run_cmd                          # looked up here, so that a test can replace run_cmd
    sys_version = tuple((running or host_version())[:3])
    if if_needed:
        pins = json.loads((HERE / "pins.json").read_text())["python"]
        if tuple(pins["min"]) <= sys_version[:2] < tuple(pins["max_exclusive"]):
            return result("ok", "host_python_ok:%d.%d.%d" % sys_version)
    got = usable(root, run)
    if got:
        return result("ok", "cached:" + PY_VERSION, root, False, PY_VERSION)
    prefix, env = prefix_of(root), build_env(environ)
    names, problem = package_policy(run)
    if problem:
        return result("NOT_RUN", problem)
    # the anchored tarball FIRST: an artifact the trust manifest does not hold, or a pin that disagrees, is refused before apt-get touches anything
    work = prefix.with_name(PRIVATE_DIR + ".dl")
    work.mkdir(parents=True, exist_ok=True)
    tgz = work / ("Python-%s.tgz" % PY_VERSION)
    ok, why = fetch_source(run, tgz, build_env(environ, trust=True))
    if not ok:
        return result("NOT_RUN", why)
    absent = missing_packages(run, names)
    if absent:
        if (os.geteuid() == 0) if root_user is None else root_user:
            install_packages(run, absent, build_env(environ, apt=True))
            absent = missing_packages(run, names)
        else:
            return result("NOT_RUN", "build_deps_missing_and_not_root:" + ",".join(absent))
    if absent:
        return result("NOT_RUN", "build_deps_missing:" + ",".join(absent))
    started = clock()
    build_dir = prefix.with_name(PRIVATE_DIR + ".build")
    try:
        src = extract_source(tgz, build_dir)
    except ValueError as exc:
        shutil.rmtree(build_dir, ignore_errors=True)
        return result("NOT_RUN", "source_refused:" + str(exc))
    if prefix.exists():
        shutil.rmtree(prefix)                   # ours (<root>/python312): a half-made build, or another recipe's, is replaced; the marker is written last
    n = jobs or default_jobs()
    try:
        for step, argv, cwd in recipe(prefix, src, n):
            rc, text, timed_out = run(argv, env=env, timeout=TIMEOUT_S[step], cwd=str(cwd))
            if rc != 0:
                shutil.rmtree(prefix, ignore_errors=True)
                return result("FAIL", "%s_%s:%s" % (step, "timeout" if timed_out else "failed", tail(text)))
        got = probe(interpreter_of(root), run)
        if not got or got["missing"] or ".".join(map(str, got["version"])) != PY_VERSION:
            shutil.rmtree(prefix, ignore_errors=True)
            why = ("modules_missing:" + ",".join(got["missing"])) if got and got["missing"] else ("wrong_version:" + ".".join(map(str, got["version"]))) if got else "does_not_run"
            return result("FAIL", "built_python_unusable:" + why)
        marker = {"schema": MARKER_SCHEMA, "version": PY_VERSION, "artifact": ARTIFACT, "recipe": recipe_id(prefix), "source_sha256": sha256_file(tgz), "jobs": n,
                  "built_epoch": int(clock()), "build_seconds": round(clock() - started, 1)}
        (prefix / MARKER).write_text(json.dumps(marker, indent=1, sort_keys=True) + "\n")
    finally:
        shutil.rmtree(build_dir, ignore_errors=True)
    return result("ok", "built:%s:%ds" % (PY_VERSION, int(clock() - started)), root, True, PY_VERSION)


# ------------------------------------------------------------------------------------------------ the manifest entry (made where there is network)
def pin_entry(sha256, size, measured):
    """The trust-manifest entry for the tarball: an upstream pin of the sha256 of a SOURCE, re-checkable at the release page. The only anchor kf_trust can hold for this
    artifact: python.org publishes an MD5 (HTML), a GPG signature and a Sigstore bundle, none of which kf_trust evaluates."""
    return {ARTIFACT: {"row": None, "kind": "file", "routing": "helper", "origin": ORIGIN % (PY_VERSION, PY_VERSION), "runs": "build",
                       "consumers": ["scripts/apps/mlrows/private_python.py"],
                       "anchors": [{"class": "upstream-pin", "sha256": sha256, "size": size, "pin_origin": "measured-from-upstream", "scope": "source",
                                    "pin_source_url": RELEASE_PAGE % PY_VERSION.replace(".", ""), "status": "MEASURED", "measured": measured}]}}


def cmd_pin_entry(a):
    data = Path(a.tarball).read_bytes()
    if a.md5 and hashlib.md5(data).hexdigest() != a.md5.lower():             # the release page's own number: a cross-check of this download, never an anchor
        print("PIN_ENTRY_MD5_MISMATCH: the file is not what the release page lists", file=sys.stderr)
        return 3
    print(json.dumps(pin_entry(hashlib.sha256(data).hexdigest(), len(data), a.measured or datetime.date.today().isoformat()), indent=1, sort_keys=True))
    return 0


def cmd_ensure(a):
    r = ensure(a.root, if_needed=a.if_needed, jobs=a.jobs)
    print("PRIVATE_PYTHON " + kv(status=r["status"], reason=r["reason"][:160], version=r["version"], python=r["python"]), flush=True)
    return 3 if r["status"] == "FAIL" else 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="verb", required=True)
    e = sub.add_parser("ensure")
    e.add_argument("--root", required=True)
    e.add_argument("--if-needed", action="store_true", help="do nothing when the Python that runs this is already inside the pinned range")
    e.add_argument("--jobs", type=int)
    p = sub.add_parser("pin-entry")
    p.add_argument("--tarball", required=True)
    p.add_argument("--md5")
    p.add_argument("--measured", help="YYYY-MM-DD, default today")
    a = ap.parse_args(argv)
    return {"ensure": cmd_ensure, "pin-entry": cmd_pin_entry}[a.verb](a)


if __name__ == "__main__":
    sys.exit(main())
