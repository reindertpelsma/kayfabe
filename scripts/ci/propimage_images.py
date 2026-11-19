#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The generic proprietary-image runner: any number of Windows images, each with its own applications (stdlib only).

STATUS: DESIGN-ONLY, 2026-11-16 (phase 1 of docs/design/V3_PROPIMAGE_RUNNER.md; GPU-free tests only: test_propimage_images.py; never run on a
box). Entered through `propimage_apps.py run --image KIND:SOURCE ...` / `--propimage PATH[:sha256]` / `--images-file FILE`; the legacy
`run` (no image flag: the one pinned image, the seven applications, record schema 1) is unchanged.

Per image, in order (each step is a refusal point with a name, in that image's record; the other images still run):
  1. the source becomes a local directory or disk (file, https URL with sha256, or a pin of scripts/ci/propimage_apps*_pins.json);
  2. with a state.json (inside the directory or zip, manifest-verified): the DETERMINISTIC shape: propimage_check.py, fixed identity, the
     recorded clock and the network off (asserted). Without one: the PLAIN shape: propimage_plain.py, a throwaway overlay of the disk,
     the real clock, the network off unless the image says `net: on` (asserted);
  3. the applications: registry definitions (propimage_registry.py) intersected with what the image declares (spec `present`, state.json
     `software`, a guest probe, or the `--apps` list); no driver or not on the image = NOT RUN, never an error;
  4. boots (grouped by the applications' timeouts, `solo` apps alone), the host kernel log in windows per application, per-app
     verdicts; the evidence kind a PASS must carry comes from the definition;
  5. cleanup of what this run created (downloads, unpacked images, overlays). An input file or directory is never written or deleted.
Kind `built` (install an ISO and install applications) is REFUSED by name here: BUILT_NOT_IMPLEMENTED (phase 2).

The record is printed as one `KF_PROPIMAGE_RUN_RESULT {json}` line (schema kayfabe.propimage-run/1) and written to --out.
"""
import datetime as dt
import hashlib
import json
import os
import shutil
import sys
import time
import urllib.request
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
WIN = HERE.parents[0] / "bench/windows"
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(WIN))
import subprocess  # noqa: E402
import clocknet_assert as cna  # noqa: E402
import propimage_apps as pa  # noqa: E402
import propimage_registry as R  # noqa: E402
import propimage_runspec as S  # noqa: E402

SCHEMA = "kayfabe.propimage-run/1"
RECORD_BYTES = 49152
FAILURE_CLASSES = ("CRASH", "HANG", "TDR", "XID", "ASSERT", "OUTPUT_MISMATCH")
PINS_GLOB = "propimage_apps*_pins.json"
MAX_URL_BYTES = 200 * 1024 ** 3
PLAIN_ARGS = ("--superposition-run", "--fb-mb")            # the harness switches propimage_plain.py understands
SWTPM_MIN = (0, 10, 0)
SWTPM_ROLES = ("libtpms", "swtpm")                          # the order the recipe builds them in
URLS_SCHEMA = "kayfabe.download-urls/1"
# Route git of the swtpm build (2026-11-17): the commit that upstream's tag v0.10.1 points at, per project: `git ls-remote <url> 'refs/tags/v0.10.1^{}'`
# (annotated tags: the peeled commit, not the tag object). Fetched by commit and verified with `git rev-parse HEAD`. These are not sha256 pins of a
# built binary (there is none); the GitHub archive tarballs are not used standalone (MEASURED 2026-11-16 on a Vast box: the download fails the size check).
SWTPM_GIT = {"libtpms": ("https://github.com/stefanberger/libtpms", "56c9f8b0b816391135f5a3112cb2feac4298caac"),
             "swtpm": ("https://github.com/stefanberger/swtpm", "53841482b0a9a1dfe63a120b00283acfe588ee72")}


# ---------------------------------------------------------------- sources
def all_pins(directory=HERE, extra=None):
    """{ref: item} over every pins file next to this module (a definition set may add its own file; refs must be unique among them).
    `extra` is the deployment's own pins file (`--pins FILE`: YOUR objects, YOUR sha256 and size, the same format): its refs ADD to the list and
    WIN over a repository ref of the same name, because the repository's pins describe the repository's own deployment, not yours."""
    out = {}
    for path in sorted(Path(directory).glob(PINS_GLOB)):
        for item in pa.load_pins(path)["items"]:
            ref = item.get("ref")
            if ref:
                # the same object may be pinned by two image sets (the signed boot GOP is shared); two different objects under one ref are refused
                if ref in out and any(out[ref].get(k) != item.get(k) for k in ("sha256", "size", "name", "dest")):
                    raise pa.Refused("PIN_DUPLICATE")
                out.setdefault(ref, dict(item, pins_file=path.name))
    if extra is not None:
        mine = {}
        for item in pa.load_pins(extra)["items"]:
            ref = item.get("ref")
            if ref:
                if ref in mine:
                    raise pa.Refused("PIN_DUPLICATE")
                mine[ref] = dict(item, pins_file=Path(extra).name)
        out.update(mine)
    return out


def failure_class(app, boot_rc):
    """CRASH / HANG / TDR / XID / ASSERT / OUTPUT_MISMATCH of a discovery row that did not pass, from what the harness and the host left."""
    ge = app["guest_events"]
    if boot_rc == 124 and app["verdict"] in ("MISSING", "HARNESS"):
        return "HANG"
    if app["host_xid"]["count"]:
        return "XID"
    if ge["tdr_4101"] or ge["live_kernel_events"] or ge["nvlddmkm_153"]:
        return "TDR"
    if "assert" in app["detail"].lower():
        return "ASSERT"
    if app["verdict"] in ("MISSING", "HARNESS") or boot_rc not in (0, None):
        return "CRASH"
    return "OUTPUT_MISMATCH"


def utc_now():
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def host_driver(ctx):
    """The host's NVIDIA driver version: ctx.hardware (from the instruction or the caller) first, else `nvidia-smi`; None, never invented."""
    given = (ctx.hardware or {}).get("driver_version")
    if given:
        return pa.clean(given, 32)
    try:
        done = ctx.runner(["nvidia-smi", "--query-gpu=driver_version", "--format=csv,noheader"], capture_output=True, text=True, timeout=30)
        line = (done.stdout or "").strip().splitlines()[0].strip() if done.returncode == 0 and (done.stdout or "").strip() else ""
    except (OSError, subprocess.SubprocessError, IndexError, AttributeError):
        return None
    return line if pa.re.fullmatch(r"\d+\.\d+(\.\d+)?", line) else None


def result_rows(rec, facts):
    """One stable-keyed row per application of every image, for comparing runs across a matrix (die, drivers, revision, image, app, lane,
    result, failure class, reason, duration, UTC start and end). A value that is not known is null."""
    out = []
    for img in rec["images"]:
        base = dict(facts, image=img["label"], guest_driver=img.get("guest_driver"), platform=rec["platform"])
        for a in img["apps"]:
            boot = next((b for b in img["boots"] if a["app"] in b["apps"]), {})
            ok = a["verdict"] == "PASS"
            out.append(dict(base, app=a["app"], row="%s:%s" % (img["label"], a["app"]), lane=a["lane"], result="PASS" if ok else "FAIL",
                            failure_class=None if ok else failure_class(a, boot.get("rc")), reason=None if ok else (a["detail"] or "no reason recorded")[:120],
                            duration_s=a["secs"] or boot.get("secs"), started_utc=boot.get("started_utc"), ended_utc=boot.get("ended_utc")))
        for n in img["not_run"]:
            out.append(dict(base, app=n["app"], row="%s:%s" % (img["label"], n["app"]), lane="gate", result="SKIPPED", failure_class=None,
                            reason=n["class"] + ":" + n["reason"], duration_s=None, started_utc=None, ended_utc=None))
        if img["verdict"] == "SKIPPED" and not img["apps"] and not img["not_run"]:
            out.append(dict(base, app=None, row=img["label"], lane="gate", result="SKIPPED", failure_class=None, reason=img["skip_class"] + ":" + img["reason"],
                            duration_s=None, started_utc=None, ended_utc=None))
    return out[:96]


def classify_discovery(app, label, boot_rc):
    """A discovery row's outcome: PASS, or BREAKAGE with a table row {row, class, reason}. Never fatal by itself (the expected-fail list and its
    ratchet are NOT built: docs/design/V3_PROPIMAGE_RUNNER.md, Later)."""
    if app["verdict"] == "PASS":
        app["outcome"] = "PASS"
        return None
    app["outcome"] = "BREAKAGE"
    return {"row": "%s:%s" % (label, app["app"]), "class": failure_class(app, boot_rc), "reason": (app["detail"] or "no reason recorded")[:120],
            "host_xid": app["host_xid"]["count"]}


def fetch_url(url, dest, sha256=None, opener=urllib.request.urlopen):
    """Streams an https URL to `dest` (bounded, sha256-checked when given). -> the sha256 of the bytes."""
    h, n = hashlib.sha256(), 0
    with opener(url, timeout=120) as r, open(dest, "wb") as f:
        for chunk in iter(lambda: r.read(1 << 22), b""):
            n += len(chunk)
            if n > MAX_URL_BYTES:
                raise pa.Refused("SOURCE_TOO_LARGE")
            h.update(chunk)
            f.write(chunk)
    got = h.hexdigest()
    if sha256 and got != sha256:
        raise pa.Refused("SOURCE_SHA256_MISMATCH")
    return got


class Local:
    """What one image became on this box. `created` are the paths this run made (the only ones it may delete)."""

    def __init__(self):
        self.kind = None          # "state" | "plain"
        self.dir = None           # state: the verified directory (image.qcow2, state.json, manifest.json)
        self.disk = None          # plain: the disk file
        self.software = None      # state.json `software` names, or None
        self.state = None
        self.created = []
        self.sha256 = None        # of the source file when it was hashed or verified


class Hooks:
    """Everything with a side effect that the tests replace."""

    def __init__(self, pi_check=None, fetch=fetch_url, sha256=S.sha256_file, unpack=None, load_dir=None):
        self.pi_check, self.fetch_url, self.sha256_file = pi_check, fetch, sha256
        self._unpack, self._load_dir = unpack, load_dir

    def unpack(self, archive, directory):
        return (self._unpack or self.pi_check.load_image)(archive, directory)

    def load_dir(self, directory):
        return (self._load_dir or (lambda d: self.pi_check.load_image(None, d)))(directory)


def tree_size(path):
    """Bytes of the regular files under a directory (no symlinks followed)."""
    return sum(f.stat().st_size for f in Path(path).rglob("*") if f.is_file() and not f.is_symlink())


def settle_identity(srec, pin, computed, enforced, expect):
    """The ONE place an identity pin is judged (docs/design/V3_PINS_ARE_DEPLOYMENT_DATA.md).

    `pin` is the sha256 the spec or the deployment names for this object (or None), `computed` the sha256 of the bytes that were read (None for a
    directory, which has no single hash), `enforced` is `propimage_runspec.pins_enforced(spec)`, `expect` the user's own `--expect-sha256`.
    The record `srec` always gets what was computed and the verdict of the pin: identity none (no pin), match, differs, unverifiable.
    A pin that differs or cannot be checked refuses ONLY when enforced; a standalone run records it and goes on. An `expect` refuses always."""
    srec["pin_sha256"] = pin
    if pin is None:
        srec["identity"] = "none"
    elif computed is None:
        srec["identity"] = "unverifiable"
    else:
        srec["identity"] = "match" if computed == pin else "differs"
    srec["expect_sha256"] = expect
    if expect is not None:
        if computed is None:
            raise pa.Refused("EXPECT_SHA256_ON_DIRECTORY")
        if computed != expect:
            raise pa.Refused("EXPECT_SHA256_MISMATCH")
    if enforced and srec["identity"] == "differs":
        raise pa.Refused("SOURCE_SHA256_MISMATCH")
    if enforced and srec["identity"] == "unverifiable":
        raise pa.Refused("SHA256_ON_DIRECTORY")


def materialize(ctx, img, base, pins, logs, hooks, srec=None, enforced=True, expect=None):
    """Source -> Local. Every failure is a pa.Refused with a name. `srec` (the image record's `source`) is filled as soon as a hash is known, so a
    refused run still says what it read; `enforced` and `expect`: see settle_identity. The defaults are the strict behaviour."""
    loc, src = Local(), img["disk"]
    srec = srec if srec is not None else {}
    base.mkdir(parents=True, exist_ok=True)
    if src["from"] == "file":
        path = Path(src["ref"])
        if not path.exists():
            raise pa.Refused("SOURCE_MISSING")
    elif src["from"] == "url":
        path = base / "dl" / "source.bin"
        path.parent.mkdir(exist_ok=True)
        loc.created.append(base / "dl")
        try:
            # strict: the pin is checked while the bytes arrive; standalone: nothing is asserted, the hash is only computed
            loc.sha256 = hooks.fetch_url(src["ref"], path, src["sha256"] if enforced else None)
        except OSError:
            raise pa.Refused("DOWNLOAD") from None
    else:   # pin: a named object of a deployment's pins file; its sha256 and size are the transfer manifest (the downloader verifies them)
        item = pins.get(src["ref"])
        if item is None:
            raise pa.Refused("PIN_UNKNOWN")
        if item["sha256"] is None or item["size"] is None:
            raise pa.Refused("PIN_UNSET:" + item["role"])
        if enforced and src["sha256"] and src["sha256"] != item["sha256"]:
            raise pa.Refused("PIN_SHA256_MISMATCH")
        dl = base / "dl"
        dl.mkdir(exist_ok=True)
        loc.created.append(dl)
        mpath = base / "manifest.json"
        mpath.write_text(json.dumps(pa.build_manifest({"items": [item]}, False, roles=(item["role"],))))
        pa.fetch(ctx, mpath, dl, logs)
        path, loc.sha256 = dl / item["dest"], item["sha256"]
    if path.is_dir():
        srec["size"] = tree_size(path)
        settle_identity(srec, src["sha256"], None, enforced, expect)
        return _from_dir(img, path, loc, hooks)
    srec["size"] = path.stat().st_size
    if loc.sha256 is None:
        loc.sha256 = hooks.sha256_file(path)                 # always: the record carries what was read, pinned or not
    srec["sha256"] = loc.sha256
    settle_identity(srec, src["sha256"], loc.sha256, enforced, expect)
    if zipfile.is_zipfile(path):
        with zipfile.ZipFile(path) as z:
            names = set(z.namelist())
        if img["state"] == "none" or not {"image.qcow2", "state.json", "manifest.json"} <= names:
            raise pa.Refused("ZIP_UNKNOWN")           # phase 1: only the three-member image zip
        out = base / "unpacked"
        loc.created.append(out)
        try:
            hooks.unpack(str(path), str(out))
        except Exception as exc:  # noqa: BLE001  propimage raises PropError, zipfile and json errors
            raise pa.Refused("UNPACK", pa.clean(exc, 120)) from None
        return _from_dir(img, out, loc, hooks)
    if img["state"] == "auto" and img["state_file"] is not None:
        return _synth_dir(img, path, loc, base, hooks, enforced, srec)
    loc.kind, loc.disk = "plain", path
    return loc


def _from_dir(img, d, loc, hooks):
    if img["state"] == "none" or not (d / "state.json").is_file():
        if not (d / "image.qcow2").is_file():
            raise pa.Refused("NO_DISK_IN_DIRECTORY")
        loc.kind, loc.disk = "plain", d / "image.qcow2"
        return loc
    if not (d / "manifest.json").is_file():
        raise pa.Refused("STATE_NO_MANIFEST")          # a state.json without its manifest cannot be verified; `--state none` runs it plain
    try:
        loc.state = hooks.load_dir(str(d))
    except Exception as exc:  # noqa: BLE001
        raise pa.Refused("STATE_INVALID", pa.clean(exc, 100)) from None
    loc.kind, loc.dir = "state", d
    loc.software = R.declared_names(loc.state.get("software")) if loc.state.get("software") else None
    return loc


def _synth_dir(img, disk, loc, base, hooks, enforced=True, srec=None):
    """A bare disk plus an explicit state.json: a directory the harness accepts (the manifest is made here, from the two files)."""
    sf = img["state_file"]
    if sf["from"] != "file" or not Path(sf["ref"]).is_file():
        raise pa.Refused("STATE_FILE_MISSING")
    d = base / "synth"
    d.mkdir(parents=True, exist_ok=True)
    loc.created.append(d)
    (d / "image.qcow2").symlink_to(Path(disk).resolve())
    shutil.copyfile(sf["ref"], d / "state.json")
    members = {"image.qcow2": hooks.sha256_file(d / "image.qcow2"), "state.json": hooks.sha256_file(d / "state.json")}
    if srec is not None:
        srec["state_sha256"] = members["state.json"]                  # the user's state.json: recorded always, pinned only when enforced
    if enforced and sf["sha256"] and sf["sha256"] != members["state.json"]:
        raise pa.Refused("STATE_FILE_SHA256_MISMATCH")
    (d / "manifest.json").write_text(json.dumps({"manifest_version": 1, "members": members}))
    return _from_dir(img, d, loc, hooks)


# ---------------------------------------------------------------- the boot plan
def plan_boots(rows, reg, budget=pa.BOOT_BUDGET_S):
    """Apps to run -> boots: per group, greedy by timeout up to `budget`; `solo` apps alone after the others. [[app, ...], ...]"""
    run = [r["app"] for r in rows if r["action"] == "run"]
    discovery = [r["app"] for r in rows if r["action"] == "run" and r.get("lane") == "discovery"]
    run = [a for a in run if a not in discovery]
    boots = []
    for group in sorted({reg[a]["group"] for a in run}):
        cur, total = [], 0
        for a in [x for x in run if reg[x]["group"] == group and not reg[x]["solo"]]:
            if cur and total + reg[a]["timeout"] > budget:
                boots.append(cur)
                cur, total = [], 0
            cur.append(a)
            total += reg[a]["timeout"]
        if cur:
            boots.append(cur)
    # solo apps and every discovery row boot alone, last, each in its own fresh clone: a crash or hang takes only its own row
    return boots + [[a] for a in run if reg[a]["solo"]] + [[a] for a in discovery]


def merged_args(apps, reg):
    """The harness words of the apps of one boot (`--flag` or `--flag value`), each flag once; two values for one flag are refused."""
    out = []
    for a in apps:
        words, i = reg[a]["harness_args"], 0
        while i < len(words):
            val = words[i + 1] if i + 1 < len(words) and not words[i + 1].startswith("--") else None
            span = [words[i]] if val is None else [words[i], val]
            if words[i] in out:
                if val is not None and out[out.index(words[i]) + 1] != val:
                    raise pa.Refused("HARNESS_ARGS_CONFLICT", words[i])
            else:
                out += span
            i += len(span)
    return out


def state_command(check_py, a, extra):
    return [sys.executable, "-B", "-I", str(check_py), "--dir", a["dir"], "--work", a["work"], "--out", a["out"], "--qemu", a["qemu"],
            "--flags", a["flags"], "--broker", a["broker"], "--gop-efi", a["gop"], "--display", "broker", "--apps", ",".join(a["apps"]), *extra]


def plain_command(plain_py, a, extra):
    return [sys.executable, "-B", "-I", str(plain_py), "--disk", a["disk"], "--work", a["work"], "--out", a["out"], "--qemu", a["qemu"],
            "--flags", a["flags"], "--apps", ",".join(a["apps"]) if a["apps"] else "probe", "--net", a["net"], "--ssh-user", a["ssh"]["user"],
            "--ssh-key", a["ssh"]["key_file"], "--ssh-port", str(a["ssh"]["port"]), *extra]


def proof(work, mode, want_net="off"):
    """What the harness wrote to run/last_cmd.txt, parsed again by clocknet_assert: {clock, net, ok}. The state shape needs the network off
    and the overlay (pa.offline_proof); the plain shape needs the real clock and the network the image asked for."""
    try:
        words = cna.command_words((Path(work) / "run/last_cmd.txt").read_text().splitlines()[0])
        clock, net = cna.assert_argv(words, "propimage_images " + mode)
    except (OSError, IndexError, ValueError):
        return {"clock": None, "net": None, "ok": False}
    got = {"clock": "recorded" if clock.faked else "real", "net": "off" if net.off else "on"}
    got["ok"] = (pa.offline_proof(work) and net.off) if mode == "ci" else (not clock.faked and got["net"] == want_net)
    return got


# ---------------------------------------------------------------- one image
def new_image_record(img):
    src = img["disk"] or img["iso"]
    ref = src.get("ref") or ""
    name = Path(ref).name if src["from"] == "file" else ref.split("?")[0].rsplit("/", 1)[-1]
    return {"label": img["label"], "kind": img["kind"], "mode": None, "source": {"from": src["from"], "name": pa.clean(name, 60), "sha256": None, "size": None, "pin_sha256": None, "identity": "none", "expect_sha256": None},
            "verdict": "RETRY", "reason": "INTERRUPTED", "skip_class": None, "guest_driver": None, "decided": None, "boots": [], "apps": [], "not_run": [], "breakages": [], "unsupported": []}


def skip(rec, reason):
    """The image did not run for a reason of the skip classes: SKIPPED, never a failure by itself (a strict run turns it into one)."""
    rec["verdict"], rec["reason"], rec["skip_class"] = "SKIPPED", reason, S.skip_class(reason)


def not_run(app, reason):
    return {"app": app, "reason": reason, "class": S.skip_class(reason)}


class _NoMark:
    def line(self, text):
        pass


def copy_evidence_for(out, evidence_dir, apps, label):
    for app in apps:
        for d in pa.run_dirs(out, app):
            for name in pa.EVIDENCE_COPY:
                src = d / name
                if src.is_file() and not src.is_symlink() and src.stat().st_size <= 1 << 20:
                    dst = Path(evidence_dir) / label / app
                    dst.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(src, dst / name)


GOP_ROLE = "gop"


def fetch_boot_gop(ctx):
    """The signed boot GOP driver (pins role `gop`) through the download manifest, the way the legacy stage fetches it next to its image. Its sha256 and
    size come from the pins file (`--pins FILE` for your own), its private signed ref must be in the URL file, and the downloader verifies the bytes.
    -> the file's path under <prefix>/gop. A named pa.Refused: PIN_UNSET:gop (no gop item, or its sha256 or size is null: nothing is fetched) or DOWNLOAD."""
    pins = pa.load_pins(ctx.pins_path)
    item = next((i for i in pins["items"] if i["role"] == GOP_ROLE), None)
    if item is None:
        raise pa.Refused("PIN_UNSET:" + GOP_ROLE)
    manifest = pa.build_manifest({"items": [item]}, False, roles=(GOP_ROLE,))
    work, dl = ctx.prefix / "work", ctx.prefix / "gop"
    for d in (work, dl):
        d.mkdir(parents=True, exist_ok=True)
    (work / "gop_manifest.json").write_text(json.dumps(manifest))
    pa.fetch(ctx, work / "gop_manifest.json", dl, ctx.prefix / "logs")
    path = dl / item["dest"]
    if not path.is_file():
        raise pa.Refused("DOWNLOAD")
    return path


class GopFile:
    """The boot GOP of one run, asked for by the first state-shaped image that is about to boot (a plain disk takes none). The caller's own file
    (`--gop-efi`) is returned as it is and nothing is fetched; otherwise the pinned one is fetched ONCE and every image gets that file. A refusal is
    remembered too: the second image does not try again."""

    def __init__(self, ctx, explicit=None):
        self.ctx, self.path, self.refusal = ctx, explicit, None

    def __call__(self):
        if self.path is None and self.refusal is None:
            try:
                self.path = fetch_boot_gop(self.ctx)
            except pa.Refused as r:
                self.refusal = r.reason
        if self.refusal is not None:
            raise pa.Refused(self.refusal)
        return self.path


def run_image(ctx, img, index, reg, pins, flags, broker, hooks, klog, evidence_dir, cpu_rows, gop, enforced=True, expect=None):
    """One image's record. Never raises for a named refusal: the record says so. `gop` is a GopFile (run_images): called, it gives the boot GOP's path."""
    rec = new_image_record(img)
    base, logs, loc = ctx.prefix / ("img%d" % (index + 1)), ctx.prefix / "logs", None
    try:
        if img["kind"] == "built":
            raise pa.Refused("BUILT_NOT_IMPLEMENTED")        # phase 2: install an ISO, then the applications
        if img["disk"]["from"] == "pin" and img["state"] != "none":
            gop()                                            # a state image of the deployment's own pins: a wrong GOP pin must not cost the image download first
        loc = materialize(ctx, img, base, pins, logs, hooks, rec["source"], enforced, expect)
        rec["mode"] = "ci" if loc.kind == "state" else "plain"
        guest = (loc.state or {}).get("qemu", {}).get("guest_driver") if type(loc.state) is dict else None
        rec["guest_driver"] = guest if type(guest) is str and pa.re.fullmatch(r"\d+\.\d+(\.\d+)?", guest) else None
        if loc.kind == "state":
            bad = pa.refusal_of([r for r in cpu_rows if r["name"] in ("cpu_vendor", "cpu_features")])
            if bad:
                if bad[1] == "CPU_VENDOR":
                    skip(rec, "CPU_VENDOR")                 # this host cannot run the fixed CPU profile: a constraint, not a defect
                else:
                    rec["verdict"], rec["reason"] = bad
                return rec
            if img["net"] == "on":
                raise pa.Refused("NET_ON_WITH_STATE")          # the state shape fakes the clock: the network stays off
        elif img["ssh"] is None:
            raise pa.Refused("SSH_REQUIRED")                   # a disk without state.json: say how to log in
        plan, unsupported, decided = S.plan_apps(img, reg, declared=loc.software)
        if decided == "NEEDS_PROBE" and loc.kind == "state":
            raise pa.Refused("NO_APP_DECLARATION")            # state.json lists no software: declare apps (--app ID=present or --apps)
        rec["decided"], rec["unsupported"] = decided, unsupported
        rec["not_run"] = [not_run(r["app"], r["reason"]) for r in plan if r["action"] == "not_run"]
        lane_of = {r["app"]: r.get("lane", "gate") for r in plan}
        boots = [None] if decided == "NEEDS_PROBE" else plan_boots(plan, reg)       # plain, nothing declared: one boot that probes the guest
        rec["boots"] = [{"index": i + 1, "apps": b or [], "rc": None, "secs": 0, "clock": None, "net": None, "proven": False,
                         "started_utc": None, "ended_utc": None}
                        for i, b in enumerate(boots)]
        if not boots:
            skip(rec, rec["not_run"][0]["reason"] if rec["not_run"] else "NO_APPS")
            return rec
        gop_path = gop() if loc.kind == "state" else None    # once per run; the first state image that has something to boot asks
        for d in (base / "work", base / "out", ctx.prefix / "kernel"):
            d.mkdir(parents=True, exist_ok=True)
        timed_out = False
        for boot in rec["boots"]:
            label, apps = "i%d_boot%d" % (index + 1, boot["index"]), boot["apps"]
            need = sum(reg[a]["timeout"] for a in apps) if apps else 3 * 3600
            if ctx.deadline_epoch and time.time() + need + 900 > ctx.deadline_epoch:
                boot["rc"], timed_out = 125, True
                continue
            out, wk = base / "out" / label, base / "work" / label
            extra = merged_args(apps, reg)
            if loc.kind == "state":
                cmd = state_command(ctx.check_py, {"dir": str(loc.dir), "work": str(wk), "out": str(out), "qemu": str(ctx.qemu), "flags": flags,
                                                   "broker": broker, "gop": str(gop_path), "apps": apps}, extra)
            else:
                unknown = [w for w in extra if w.startswith("--") and w not in PLAIN_ARGS]
                if unknown:
                    raise pa.Refused("PLAIN_HARNESS_ARGS", unknown[0])
                cmd = plain_command(ctx.plain_py, {"disk": str(loc.disk), "work": str(wk), "out": str(out), "qemu": str(ctx.qemu), "flags": flags,
                                                   "apps": apps, "net": img["net"], "ssh": img["ssh"]}, extra)
            klog.mark(label, "start")
            t0, boot["started_utc"] = time.monotonic(), utc_now()
            boot["rc"] = (ctx.boot_runner or pa.run_boot)(ctx, cmd, logs / (label + ".log"), need + pa.BOOT_SLACK_S,
                                                          pa.Mark(klog, label, apps) if apps else _NoMark())
            boot["secs"], boot["ended_utc"] = int(time.monotonic() - t0), utc_now()
            klog.mark(label, "end")
            got = proof(wk, rec["mode"], img["net"])
            boot.update(clock=got["clock"], net=got["net"], proven=got["ok"])
            try:
                rows_json = pa.read_json(out / "result.json")
                rows_json = rows_json if type(rows_json) is list else []
            except (OSError, ValueError):
                rows_json = []
            ran = apps or [str(r["step"])[4:] for r in rows_json if type(r) is dict and str(r.get("step", "")).startswith("app:")]
            boot["apps"] = ran
            per, unattributed = pa.xid_by_window(klog, ran, label) if apps else ({}, [])
            for app in ran:
                if app not in reg:
                    rec["not_run"].append(not_run(app, "NO_DRIVER"))
                    continue
                item = pa.collect_app(out, app, rows_json, per.get(app, []), expected=reg[app]["kind"])
                if item["verdict"] == "NOT_RUN":
                    rec["not_run"].append(not_run(app, "NOT_RUN"))
                    continue
                item["lane"] = lane_of.get(app, "gate")
                if not got["ok"] and item["verdict"] == "PASS":
                    item["verdict"], item["detail"], item["lane"] = "FAIL", "CLOCK_NET_NOT_PROVEN: " + item["detail"][:150], "gate"   # never excused as a discovery row
                if item["lane"] == "discovery":
                    broken = classify_discovery(item, img["label"], boot["rc"])
                    rec["breakages"] += [broken] if broken else []
                else:
                    item["outcome"] = "PASS" if item["verdict"] == "PASS" else "FAIL"
                rec["apps"].append(item)
            if unattributed:
                boot["host_xid_unattributed"] = {"count": len(unattributed), "lines": [t for _, t in unattributed[:pa.MAX_XID_LINES]]}
            if evidence_dir:
                copy_evidence_for(out, evidence_dir, ran, img["label"])
        rec["verdict"], rec["reason"] = ("RETRY", "TIME") if timed_out else grade_image(rec)
    except pa.Refused as r:
        if r.reason in S.SKIP_CLASS:                          # missing input, nothing declared: skipped with its class
            skip(rec, r.reason)
        else:
            rec["verdict"], rec["reason"] = "REFUSED", r.reason
    except (ValueError, OSError, KeyError) as exc:
        rec["verdict"], rec["reason"], rec["error"] = "REFUSED", "INTERNAL", pa.clean(exc, 120)
    finally:
        for p in (loc.created if loc else []):                 # only what this run made; an input is never touched
            shutil.rmtree(p, ignore_errors=True)
        shutil.rmtree(base / "work", ignore_errors=True)
    return rec


def grade_image(rec):
    """PASS: at least one app ran and every GATE app that ran is PASS (discovery rows are reported, never graded; an app of a boot whose clock and network were not proven from its argv was
    already turned into a FAIL when it was collected)."""
    if not rec["apps"]:
        if any(b["rc"] not in (0, None) for b in rec["boots"]):
            return "RETRY", "HARNESS"
        return "SKIPPED", (rec["not_run"][0]["reason"] if rec["not_run"] else "NO_APPS")
    apps = [a for a in rec["apps"] if a["lane"] == "gate"]      # a discovery row never decides the image: its failures are the breakages table
    bad = [a for a in apps if a["verdict"] != "PASS"]
    hard = [a for a in bad if a["verdict"] in ("FAIL", "INCONCLUSIVE", "LAUNCH_ONLY")]
    if hard:
        return "FAIL", "APP_FAIL:" + hard[0]["app"]
    if bad:
        return "RETRY", "HARNESS:" + bad[0]["app"]
    return "PASS", ""


def skipped_items(images):
    """Every skip of a run: an image that did not run at all (`app` None) and every app that was not run, each with its class."""
    out = []
    for i in images:
        if i["verdict"] == "SKIPPED" and not i["apps"]:
            out.append({"image": i["label"], "app": None, "class": i["skip_class"], "reason": i["reason"]})
        out += [{"image": i["label"], "app": n["app"], "class": n["class"], "reason": n["reason"]} for n in i["not_run"]]
    return out[:64]


def grade_run(images, strict=False):
    """(verdict, reason) of the whole run. FAIL beats RETRY beats REFUSED. A skip is NOT a failure (verdict PASS when something ran and
    everything that ran passed, SKIPPED when nothing ran) unless `strict`: then any skip is a FAIL, STRICT_SKIP:<image>:<app>:<CLASS>."""
    for want in ("FAIL", "RETRY", "REFUSED"):
        for i in images:
            if i["verdict"] == want:
                return want, ("%s:%s" % (i["label"], i["reason"]))[:80]
    if strict:
        skips = skipped_items(images)
        if skips:
            first = skips[0]
            return "FAIL", ("STRICT_SKIP:%s:%s:%s" % (first["image"], first["app"] or "-", first["class"]))[:80]
    if any(i["verdict"] == "PASS" for i in images):
        return "PASS", ""
    return "SKIPPED", "NOTHING_RAN"


def summarize(rec):
    """ran / passed / failed / skipped counts and the skips by class: the numbers of the one-line summary."""
    ran = [a for i in rec["images"] for a in i["apps"]]
    skips = skipped_items(rec["images"])
    classes = {}
    for k in skips:
        classes[k["class"]] = classes.get(k["class"], 0) + 1
    lanes = {lane: {"ran": sum(a["lane"] == lane for a in ran), "passed": sum(a["lane"] == lane and a["verdict"] == "PASS" for a in ran)} for lane in S.LANES}
    return {"ran": len(ran), "passed": sum(a["verdict"] == "PASS" for a in ran), "failed": sum(a["verdict"] != "PASS" for a in ran),
            "skipped": len(skips), "classes": classes, "lanes": lanes}


def summary_line(rec):
    """The plain sentence: ran N, passed N, skipped M (reasons)."""
    s = rec["summary"]
    why = ", ".join("%s %d" % (k, v) for k, v in sorted(s["classes"].items()))
    g, d = s["lanes"]["gate"], s["lanes"]["discovery"]
    lanes = "gate: ran %d, passed %d | discovery: ran %d, passed %d, known breakages %d" % (g["ran"], g["passed"], d["ran"], d["passed"], len(rec["breakages"]))
    return "ran %d, passed %d%s, skipped %d%s%s [%s]" % (s["ran"], s["passed"], (", failed %d" % s["failed"]) if s["failed"] else "", s["skipped"],
                                                           (" (%s)" % why) if why else "", "; STRICT: any skip is a failure" if rec["strict"] else "", lanes)


# ---------------------------------------------------------------- swtpm >= 0.10 (docs/design/V3_PROPIMAGE_RUNNER.md, 2026-11-17)
def url_file_names(path, refs):
    """True when the presigned-URL file (kayfabe.download-urls/1) carries a URL for every one of `refs`. Only the names are looked at, never a value;
    a file that is missing, not a plain small JSON file or of another shape just does not name them (the box then takes route git)."""
    p = Path(path)
    try:
        if p.is_symlink() or not p.is_file() or p.stat().st_size > 1 << 20:
            return False
        doc = json.loads(p.read_text())
    except (OSError, ValueError):
        return False
    urls = doc.get("urls") if type(doc) is dict and doc.get("schema") == URLS_SCHEMA else None
    return type(urls) is dict and all(type(urls.get(r)) is str for r in refs)


def tools_by_signed_ref(ctx, pins):
    """The pins items of libtpms and swtpm when route signed applies (the pins file has both, each with a private `ref`, and the URL file names both
    refs), else None."""
    items = {i["role"]: i for i in pins["items"] if i["role"] in SWTPM_ROLES}
    if set(items) != set(SWTPM_ROLES) or not all("ref" in i for i in items.values()):
        return None
    return items if url_file_names(ctx.urls, [i["ref"] for i in items.values()]) else None


def clone_pinned(ctx, role, work, logs):
    """`git init`, a fetch of the ONE pinned commit (depth 1) and a checkout of it; then `git rev-parse HEAD` must equal the pin. -> the directory."""
    url, commit = SWTPM_GIT[role]
    src = work / role
    src.mkdir(parents=True, exist_ok=True)
    env = dict(ctx.environ, GIT_TERMINAL_PROMPT="0", GIT_ALLOW_PROTOCOL="https")
    try:
        for argv in (["git", "init", "-q", str(src)], ["git", "-C", str(src), "fetch", "-q", "--depth", "1", url, commit],
                     ["git", "-C", str(src), "-c", "advice.detachedHead=false", "checkout", "-q", "FETCH_HEAD"]):
            if pa.run_command(ctx, argv, logs / "swtpm_source.log", 900, env=env) != 0:
                raise pa.Refused("SWTPM_SOURCE")
        done = ctx.runner(["git", "-C", str(src), "rev-parse", "HEAD"], capture_output=True, text=True, timeout=60, env=env)
    except (OSError, subprocess.SubprocessError):
        raise pa.Refused("SWTPM_SOURCE") from None
    if done.returncode != 0 or (done.stdout or "").strip() != commit:
        raise pa.Refused("SWTPM_SOURCE_COMMIT")
    return src


def build_tools(ctx, work, logs):
    """Build swtpm 0.10.1 under /opt/tpm exactly as the legacy stage does (pa.build_swtpm), from the private signed tarballs when the URL file names
    their refs (route signed; the tools pins hold in every mode: PIN_UNSET:<role> refuses before anything is fetched) and from the pinned upstream
    commits otherwise (route git). Then every later boot uses it: SWTPM and LD_LIBRARY_PATH in ctx.environ, as the legacy stage sets them.
    -> ("signed" | "git"). A named pa.Refused on every failure."""
    base = work / "swtpm"
    dl, src = base / "dl", base / "src"
    for d in (dl, src):
        d.mkdir(parents=True, exist_ok=True)
    pins = pa.load_pins(ctx.pins_path)
    items = tools_by_signed_ref(ctx, pins)
    try:
        if items:
            manifest = pa.build_manifest(pins, True, roles=SWTPM_ROLES)
            for it in manifest["items"]:
                it["sources"] = [x for x in it["sources"] if x["kind"] == "signed"]           # the GitHub archive mirror is never tried here
            (base / "manifest.json").write_text(json.dumps(manifest))
            pa.fetch(ctx, base / "manifest.json", dl, logs)
            env = pa.build_swtpm(ctx, dl, src, logs)
        else:
            env = pa.build_swtpm(ctx, dl, src, logs, trees={role: clone_pinned(ctx, role, src, logs) for role in SWTPM_ROLES})
    except OSError:
        raise pa.Refused("SWTPM_BUILD") from None                  # a program of the recipe is not on the box
    ctx.environ.update(SWTPM=pa.SWTPM_PREFIX + "/bin/swtpm", LD_LIBRARY_PATH=env["LD_LIBRARY_PATH"])
    ctx.environ.update(pa.swtpm_env(ctx.environ))
    return "signed" if items else "git"


def run_images(ctx, spec, evidence_dir=None, hooks=None, gop=None):
    """The generic stage. Returns the record; never raises for a named refusal."""
    rec = {"schema": SCHEMA, "verdict": "RETRY", "reason": "INTERRUPTED", "sha": ctx.sha, "strict": False, "pins_enforced": False, "family": None, "cpu_vendor": None,
           "preflight": [], "images": [], "summary": {"ran": 0, "passed": 0, "failed": 0, "skipped": 0, "classes": {}, "lanes": {"gate": {"ran": 0, "passed": 0}, "discovery": {"ran": 0, "passed": 0}}},
           "strict_failures": [], "breakages": [], "platform": "kf3", "rows": [], "swtpm": {"version": None, "built": False}}
    klog = None
    try:
        pa.check_prefix(ctx.prefix)
        spec = S.validate(spec)
        rec["strict"] = spec["strict"]
        enforced, expect = S.pins_enforced(spec), spec["expect_sha256"]
        rec["pins_enforced"] = enforced
        if any(i["kind"] == "built" for i in spec["images"]):
            raise pa.Refused("BUILT_NOT_IMPLEMENTED")        # phase 2: refused before anything else, on any machine
        reg = R.load_registry()
        if spec["strict"]:
            S.strict_unknown_app(spec, reg)                 # a strict instruction naming an app with no definition fails by name, before any boot
        pins = all_pins(extra=ctx.pins_path if Path(ctx.pins_path) != pa.PINS_PATH else None) if any(i["disk"] and i["disk"]["from"] == "pin" for i in spec["images"]) else {}
        hooks = hooks or Hooks(pa.harness(ctx.checkout))
        pi = hooks.pi_check.PI if hooks.pi_check else pa.harness(ctx.checkout).PI
        try:
            cpuinfo = Path(ctx.cpuinfo).read_text()
        except OSError:
            cpuinfo = ""
        rows = pa.cpu_rows(cpuinfo, pi=pi)
        rec["cpu_vendor"] = {"AuthenticAMD": "amd", "GenuineIntel": "intel"}.get(pi.parse_cpuinfo(cpuinfo)[0], "other")
        frow, rec["family"] = pa.family_row(ctx.hardware or {})
        version = pa.swtpm_version(pa.swtpm_binary(ctx.environ), ctx.runner, pa.swtpm_env(ctx.environ))
        have_swtpm = bool(version and version >= SWTPM_MIN)
        free = ctx.free_bytes if ctx.free_bytes is not None else shutil.disk_usage(pa._existing(ctx.prefix)).free
        rows += [frow,
                 pa.row("swtpm", "MATCH" if have_swtpm else "WARN",
                        ("%d.%d.%d" % version) if have_swtpm else "%s: swtpm 0.10.1 is built under %s before the first boot" % (
                            ("%d.%d.%d" % version) if version else "missing", pa.SWTPM_PREFIX)),
                 pa.row("kf3_qemu", "MATCH" if os.access(ctx.qemu, os.X_OK) else "MISMATCH", "the kf3 QEMU of this revision"),
                 pa.row("disk", "MATCH" if free >= pa.DISK_HEADROOM else "MISMATCH", "free %d GiB" % (free >> 30))]
        rec["preflight"] = rows
        hard = pa.refusal_of([r for r in rows if r["name"] not in ("cpu_vendor", "cpu_features")])
        if hard:
            rec["verdict"], rec["reason"] = hard
            return rec
        for d in ("logs", "out", "work", "gop"):
            (ctx.prefix / d).mkdir(parents=True, exist_ok=True)
        swrow = next(r for r in rows if r["name"] == "swtpm")
        if have_swtpm:
            ctx.environ.update(pa.swtpm_env(ctx.environ))
            rec["swtpm"]["version"] = "%d.%d.%d" % version
        else:
            # a box older than 0.10 (MEASURED 2026-11-17: a stock Vast Ubuntu 22.04 box has 0.6.3) is not refused: the legacy stage's build, before the first boot
            try:
                route = build_tools(ctx, ctx.prefix / "work", ctx.prefix / "logs")
            except pa.Refused as r:
                swrow.update(status="MISMATCH", detail=pa.clean("build failed: " + r.reason, 160))
                raise
            built = pa.swtpm_version(pa.swtpm_binary(ctx.environ), ctx.runner, ctx.environ)         # build_swtpm proved >= 0.10 with this environment
            rec["swtpm"] = {"version": ".".join(str(x) for x in built) if built else None, "built": True}
            swrow.update(status="MATCH", detail="built %s under %s (%s)" % (rec["swtpm"]["version"], pa.SWTPM_PREFIX,
                                                                         "private signed tarballs" if route == "signed" else "pinned upstream commits"))
        klog = pa.KernelLog(ctx.prefix / "kernel", ctx.klog_source)
        klog.start()
        broker = ctx.checkout / "broker/nvkvm-display-broker"
        if not broker.is_file() and any(i["kind"] == "prebuilt" for i in spec["images"]):
            if pa.run_command(ctx, ["make", "-C", str(ctx.checkout / "broker")], ctx.prefix / "logs/broker_build.log", 900) != 0:
                raise pa.Refused("NO_BROKER_BUILD")
        flags = str(ctx.checkout / "scripts/bench/windows/kf3_flags.sample.env")
        gop_file = GopFile(ctx, gop)
        for n, img in enumerate(spec["images"]):
            rec["images"].append(run_image(ctx, img, n, reg, pins, flags, str(broker), hooks, klog, evidence_dir, rows,
                                           gop_file, enforced, expect.get(img["label"])))
            write_record(ctx.out, rec)
        rec["verdict"], rec["reason"] = grade_run(rec["images"], spec["strict"])
        if spec["strict"]:
            rec["strict_failures"] = skipped_items(rec["images"])[:32]
    except pa.Refused as r:
        rec["verdict"], rec["reason"] = "REFUSED", r.reason
    except S.SpecError as exc:
        rec["verdict"], rec["reason"] = "REFUSED", "SPEC:" + str(exc)[:60]
    except (ValueError, OSError, KeyError, R.RegistryError) as exc:
        rec["verdict"], rec["reason"], rec["error"] = "REFUSED", "INTERNAL", pa.clean(exc, 120)
    finally:
        if klog is not None:
            klog.stop()
        shutil.rmtree(ctx.prefix / "work", ignore_errors=True)
        rec["breakages"] = [b for i in rec["images"] for b in i["breakages"]][:32]
        rec["summary"] = summarize(rec)
        rec["rows"] = result_rows(rec, {"die": (pa.clean((ctx.hardware or {}).get("gpu_name"), 64) if (ctx.hardware or {}).get("gpu_name") not in (None, "unknown") else None),
                                        "family": rec["family"], "host_driver": host_driver(ctx), "kf3_rev": ctx.sha if pa.SHA_RE.fullmatch(ctx.sha) else None})
    return rec


def write_record(path, rec):
    """Atomic; over the size bound the xid lines and evidence lists are cut (the verdicts are never cut)."""
    data = json.dumps(rec, separators=(",", ":"), sort_keys=True)
    if len(data.encode()) > RECORD_BYTES:
        for image in rec["images"]:
            for a in image["apps"]:
                a["host_xid"]["lines"], a["evidence"] = a["host_xid"]["lines"][:2], a["evidence"][:8]
        data = json.dumps(rec, separators=(",", ":"), sort_keys=True)
    tmp = Path(str(path) + ".tmp")
    tmp.write_text(data + "\n")
    tmp.replace(path)
    return data


def exit_code(rec):
    """0 PASS (ran something, everything that ran passed) and 0 SKIPPED (nothing ran, permissive); 1 FAIL; 2 REFUSED; 3 RETRY; 4 a strict run
    with a skip (STRICT_SKIP)."""
    verdict, reason = rec["verdict"], rec["reason"]
    if verdict in ("PASS", "SKIPPED"):
        return 0
    if verdict == "FAIL":
        return 4 if reason.startswith("STRICT_") else 1
    return {"RETRY": 3, "REFUSED": 2}.get(verdict, 1)


def finish(rec, out):
    rec.setdefault("summary", summarize(rec))
    data = write_record(out, rec) if out else json.dumps(rec, separators=(",", ":"), sort_keys=True)
    print("KF_PROPIMAGE_RUN_RESULT " + data, flush=True)
    print("KF_PROPIMAGE_RUN_SUMMARY verdict=%s %s" % (rec["verdict"], summary_line(rec)), flush=True)
    for i in rec.get("images", []):
        src = i["source"]
        if src["identity"] in ("differs", "unverifiable") and not rec.get("pins_enforced"):
            print("KF_PROPIMAGE_RUN_PIN_NOTE image=%s identity=%s computed=%s pin=%s (recorded, not enforced outside strict)" % (
                i["label"], src["identity"], (src["sha256"] or "-")[:16], (src["pin_sha256"] or "-")[:16]), flush=True)
    for b in rec.get("breakages", []):
        print("KF_PROPIMAGE_RUN_BREAKAGE row=%s class=%s reason=%s" % (b["row"], b["class"], b["reason"].replace(" ", "_")[:80]), flush=True)
    return exit_code(rec)
