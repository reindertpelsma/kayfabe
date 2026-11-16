#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The run spec of the proprietary-image runner: a LIST of Windows images, each with its apps (stdlib only).

STATUS: LIVE for kind `prebuilt` (phase 1), 2026-11-16; kind `built` is validated here but REFUSED by the runner (BUILT_NOT_IMPLEMENTED,
phase 2, after the release run). Design: docs/design/V3_PROPIMAGE_RUNNER.md. GPU-free tests: scripts/ci/test_propimage_runspec.py.

One shape serves the Worker (the instruction's `propimages` list holds the same image objects), a standalone JSON file
(`--images-file`) and the command line (`--image KIND:SOURCE ...`, parsed into the same object). Every refusal has a name
(`SpecError.reason`); nothing is guessed.

  {"schema": "kayfabe.propimage-images/1", "images": [IMAGE, ...], "strict": false}
  IMAGE = {"label": "name",                      optional; default image1, image2, ...
           "kind": "prebuilt" | "built",
           "disk": SOURCE,                       prebuilt: the disk (an unpacked image directory, a zip, or a bare disk file)
           "iso": SOURCE,                        built (phase 2): the install ISO
           "state": "auto" | "none",             prebuilt: use the state.json found with the disk (auto, default) or ignore it
           "state_file": SOURCE,                 prebuilt: a state.json given separately (needs state auto)
           "net": "off" | "on",                  default off; `on` is refused with a state.json (a faked clock needs the network off)
           "only": ["app", ...],                 restrict what is run to these applications
           "apps": {"app": {"mode": "present" | "skip" | "install", "source": SOURCE, "lane": "gate" | "discovery"}},
           "ssh": {"user": "...", "key_file": "path", "port": N}}      a disk without state.json: how to log in
  SOURCE = {"from": "official" | "file" | "url" | "pin", "ref": "...", "sha256": "64 hex"}   ref is not used by `official`

Two policies. STANDALONE (`strict` false, the default): run as much as the inputs allow; whatever cannot run is SKIPPED with a reason class
(MISSING_INPUT, NO_DRIVER, NOT_INSTALLED, NOT_PROVIDED, CONSTRAINT) and that is not a failure. STRICT (`strict` true, `--strict`, the Worker's
instruction): every listed image and app must run, a skip is a failure by name. A strict spec must say exactly what runs: every image lists at
least one `present` app, and `skip` and `only` entries are refused (STRICT_SKIP_ENTRY, STRICT_ONLY, STRICT_NEEDS_APPS); an app without a
definition is STRICT_UNKNOWN_APP (`strict_unknown_app`).

Two lanes per application row (`lane`, default `gate`): GATE rows must run and pass, a failure is fatal always; DISCOVERY rows may fail or
crash (exotic rows that try to break kf3): a failure never fails the run by itself and is reported in the known-breakages table, checked against
the expected-fail list (propimage_expected_fail.json) by the runner; each discovery row boots alone in a fresh throwaway clone.

Rules (each is a named refusal): `present` and `skip` take no source; `install` takes one and only a `built` image may use it (the
runner installs applications onto an image it just built); a `built` image needs `iso` and at least one `install` app, a
`prebuilt` one needs `disk`; `net: on` is refused together with an explicit state.json (the deterministic shape fakes the clock).
"""
import hashlib
import json
import re
import shlex
from pathlib import Path

SCHEMA = "kayfabe.propimage-images/1"
KINDS = ("prebuilt", "built")
FROMS = ("official", "file", "url", "pin")
MODES = ("present", "skip", "install")
LANES = ("gate", "discovery")
LABEL_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,31}")
APP_RE = re.compile(r"[a-z0-9][a-z0-9_]{0,31}")
HEX64 = re.compile(r"[0-9a-f]{64}")
PRINTABLE = re.compile(r"[\x20-\x7e]{1,1024}")
MAX_IMAGES = 8
IMAGE_KEYS = {"label", "kind", "disk", "iso", "state", "state_file", "net", "only", "apps", "ssh"}
PER_IMAGE_FLAGS = ("--label", "--state", "--net", "--apps", "--app", "--ssh-user", "--ssh-key", "--ssh-port")
IMAGE_FLAGS = ("--image", "--propimage")


class SpecError(ValueError):
    """A named refusal of the run spec. `reason` is an upper-case token with an optional `:detail`; never a value from the network."""

    def __init__(self, reason, detail=""):
        super().__init__(reason + (":" + detail if detail else ""))
        self.reason, self.detail = reason, detail


def _no_duplicates(pairs):
    out = {}
    for k, v in pairs:
        if k in out:
            raise SpecError("SPEC_DUPLICATE_KEY", str(k)[:32])
        out[k] = v
    return out


def parse_json(raw):
    try:
        return json.loads(raw, object_pairs_hook=_no_duplicates)
    except SpecError:
        raise
    except (ValueError, RecursionError):
        raise SpecError("SPEC_NOT_JSON") from None


# ---------------------------------------------------------------- validation
def check_source(src, where, allow=FROMS):
    """One source object, normalized; `ref` of `official` may be empty (the registry's pinned official source)."""
    if type(src) is not dict or not {"from"} <= set(src) <= {"from", "ref", "sha256"}:
        raise SpecError("SOURCE_BAD", where)
    if src["from"] not in allow:
        raise SpecError("SOURCE_FROM", "%s:%s" % (where, str(src["from"])[:16]))
    ref, sha = src.get("ref"), src.get("sha256")
    if src["from"] == "official":
        if ref is not None and not PRINTABLE.fullmatch(str(ref)):
            raise SpecError("SOURCE_REF", where)
    elif type(ref) is not str or not PRINTABLE.fullmatch(ref):
        raise SpecError("SOURCE_REF", where)
    if src["from"] == "url" and not re.fullmatch(r"https://[^\s]{1,1000}", ref):
        raise SpecError("SOURCE_URL_NOT_HTTPS", where)
    if sha is not None and not (type(sha) is str and HEX64.fullmatch(sha)):
        raise SpecError("SOURCE_SHA256", where)
    return {"from": src["from"], "ref": ref, "sha256": sha}


def check_image(img, index):
    if type(img) is not dict or "kind" not in img or not set(img) <= IMAGE_KEYS:
        raise SpecError("IMAGE_KEYS", str(index + 1))
    label = img.get("label", "image%d" % (index + 1))
    if type(label) is not str or not LABEL_RE.fullmatch(label):
        raise SpecError("LABEL_BAD", str(index + 1))
    kind = img["kind"]
    if kind not in KINDS:
        raise SpecError("KIND_BAD", label)
    out = {"label": label, "kind": kind, "disk": None, "iso": None, "state": img.get("state", "auto"), "state_file": None,
           "net": img.get("net", "off"), "only": [], "apps": {}, "ssh": None}
    if out["state"] not in ("auto", "none"):
        raise SpecError("STATE_BAD", label)
    if out["net"] not in ("off", "on"):
        raise SpecError("NET_BAD", label)
    if kind == "prebuilt":
        if "disk" not in img or "iso" in img:
            raise SpecError("DISK_REQUIRED" if "disk" not in img else "ISO_ON_PREBUILT", label)
        out["disk"] = check_source(img["disk"], label + ".disk", ("file", "url", "pin"))
    else:
        if "iso" not in img or "disk" in img or "state_file" in img or img.get("state", "auto") != "auto" or "ssh" in img:
            raise SpecError("ISO_REQUIRED" if "iso" not in img else "BUILT_KEYS", label)
        out["iso"] = check_source(img["iso"], label + ".iso", ("official", "file", "url"))
    if "state_file" in img:
        if out["state"] == "none":
            raise SpecError("STATE_CONFLICT", label)
        out["state_file"] = check_source(img["state_file"], label + ".state_file", ("file", "url"))
    if out["net"] == "on" and (out["state_file"] is not None):
        raise SpecError("NET_ON_WITH_STATE", label)          # explicit state.json = the deterministic shape: faked clock, network off
    only = img.get("only", [])
    if type(only) is not list or len(only) > 30 or not all(type(a) is str and APP_RE.fullmatch(a) for a in only) or len(set(only)) != len(only):
        raise SpecError("ONLY_BAD", label)
    out["only"] = only
    apps = img.get("apps", {})
    if type(apps) is not dict or len(apps) > 32:
        raise SpecError("APPS_BAD", label)
    for name, entry in apps.items():
        if not APP_RE.fullmatch(str(name)):
            raise SpecError("APP_ID_BAD", label)
        if type(entry) is not dict or "mode" not in entry or not set(entry) <= {"mode", "source", "lane"}:
            raise SpecError("APP_ENTRY_BAD", "%s:%s" % (label, name))
        mode = entry["mode"]
        if entry.get("lane", "gate") not in LANES:
            raise SpecError("APP_LANE_BAD", "%s:%s" % (label, name))
        if mode not in MODES:
            raise SpecError("APP_MODE_BAD", "%s:%s" % (label, name))
        if mode == "install":
            if "source" not in entry:
                raise SpecError("APP_SOURCE_REQUIRED", "%s:%s" % (label, name))
            if kind != "built":
                raise SpecError("INSTALL_ON_PREBUILT", "%s:%s" % (label, name))
            out["apps"][name] = {"mode": mode, "source": check_source(entry["source"], "%s.%s" % (label, name), ("official", "file", "url"))}
            if "lane" in entry:
                out["apps"][name]["lane"] = entry["lane"]
        else:
            if "source" in entry:
                raise SpecError("APP_SOURCE_FORBIDDEN", "%s:%s" % (label, name))
            if kind == "built":
                raise SpecError("BUILT_APP_MODE", "%s:%s" % (label, name))
            out["apps"][name] = {"mode": mode}
            if "lane" in entry:
                out["apps"][name]["lane"] = entry["lane"]
    if kind == "built" and not any(e["mode"] == "install" for e in out["apps"].values()):
        raise SpecError("BUILT_NEEDS_APPS", label)
    if "ssh" in img:
        ssh = img["ssh"]
        if type(ssh) is not dict or set(ssh) - {"user", "key_file", "port"} or not {"user", "key_file"} <= set(ssh):
            raise SpecError("SSH_BAD", label)
        if not re.fullmatch(r"[A-Za-z][A-Za-z0-9_.-]{0,31}", str(ssh["user"])) or not PRINTABLE.fullmatch(str(ssh["key_file"])):
            raise SpecError("SSH_BAD", label)
        port = ssh.get("port", 22)
        if type(port) is not int or not 1 <= port <= 65535:
            raise SpecError("SSH_BAD", label)
        out["ssh"] = {"user": ssh["user"], "key_file": ssh["key_file"], "port": port}
    return out


def validate(doc):
    """The normalized spec (every default filled in) or a SpecError by name."""
    if type(doc) is not dict:
        raise SpecError("SPEC_NOT_OBJECT")
    if doc.get("schema") != SCHEMA or not {"schema", "images"} <= set(doc) <= {"schema", "images", "strict"}:
        raise SpecError("SPEC_SCHEMA")
    strict = doc.get("strict", False)
    if type(strict) is not bool:
        raise SpecError("STRICT_BAD")
    images = doc["images"]
    if type(images) is not list or not 1 <= len(images) <= MAX_IMAGES:
        raise SpecError("SPEC_IMAGES")
    norm = [check_image(i, n) for n, i in enumerate(images)]
    labels = [i["label"] for i in norm]
    if len(set(labels)) != len(labels):
        raise SpecError("LABEL_DUP")
    if strict:
        for img in norm:
            if img["only"]:
                raise SpecError("STRICT_ONLY", img["label"])
            if any(e["mode"] == "skip" for e in img["apps"].values()):
                raise SpecError("STRICT_SKIP_ENTRY", img["label"])
            if not any(e["mode"] in ("present", "install") for e in img["apps"].values()):
                raise SpecError("STRICT_NEEDS_APPS", img["label"])
    return {"schema": SCHEMA, "images": norm, "strict": strict}


def validate_list(images, strict=False):
    """The Worker's form: the instruction carries the image objects as a list (and `propimages_strict`)."""
    return validate({"schema": SCHEMA, "images": list(images), "strict": bool(strict)})


def strict_unknown_app(spec, reg):
    """In a strict spec every named app needs a definition. -> SpecError("STRICT_UNKNOWN_APP", "label:app") for the first that has none."""
    import propimage_registry as R
    for img in spec["images"]:
        for name in img["apps"]:
            if R.canonical(reg, name) is None:
                raise SpecError("STRICT_UNKNOWN_APP", "%s:%s" % (img["label"], name))


# why something did not run: the five classes of a skip (a skip is not a failure unless the run is strict)
SKIP_CLASS = {"NO_DRIVER": "NO_DRIVER", "NOT_ON_IMAGE": "NOT_INSTALLED", "SKIPPED": "CONSTRAINT", "CPU_VENDOR": "CONSTRAINT",
              "SSH_REQUIRED": "MISSING_INPUT", "SOURCE_MISSING": "MISSING_INPUT", "NO_APP_DECLARATION": "NOT_PROVIDED", "NO_APPS": "NOT_PROVIDED"}


def skip_class(reason):
    """The class of a skip reason: NEEDS:<x> is MISSING_INPUT; a reason with no entry is CONSTRAINT."""
    if str(reason).startswith("NEEDS:"):
        return "MISSING_INPUT"
    return SKIP_CLASS.get(str(reason).split(":")[0], "CONSTRAINT")


# ---------------------------------------------------------------- the command line
def parse_source(text, kind_hint="file"):
    """`official`, `https://...` (url), `pin:NAME` or a path, each with an optional `#sha256=HEX` suffix."""
    sha = None
    m = re.search(r"#sha256=([0-9a-f]{64})$", text)
    if m:
        text, sha = text[:m.start()], m.group(1)
    elif "#sha256=" in text:
        raise SpecError("SOURCE_SHA256", "cli")
    if text == "official" or text.startswith("official:"):
        return {"from": "official", "ref": text[9:] or None, "sha256": sha}
    if text.startswith("pin:"):
        return {"from": "pin", "ref": text[4:], "sha256": sha}
    if text.startswith(("https://", "http://")):
        return {"from": "url", "ref": text, "sha256": sha}
    return {"from": "file", "ref": text, "sha256": sha}


def parse_app(text):
    """`ID=present`, `ID=skip`, `ID=install:SOURCE` -> (id, entry)."""
    name, sep, rest = text.partition("=")
    if not sep or not rest:
        raise SpecError("APP_FLAG_BAD", name[:32])
    lane = None
    m = re.search(r",lane=(gate|discovery)$", rest)
    if m:
        rest, lane = rest[:m.start()], m.group(1)
    extra = {"lane": lane} if lane else {}
    if rest in ("present", "skip"):
        return name, dict({"mode": rest}, **extra)
    if rest.startswith("install:"):
        src = rest[8:]
        if src.startswith("file:"):
            src = src[5:]
        elif src.startswith("url:"):
            src = src[4:]
        return name, dict({"mode": "install", "source": parse_source(src)}, **extra)
    raise SpecError("APP_FLAG_BAD", name[:32])


def split_flags(argv):  # noqa: C901
    """Takes the image flags out of a command line. -> (the remaining words, a spec dict or None). Per-image flags belong to the
    nearest `--image`/`--propimage` before them (like ffmpeg's per-input options); one before any image is refused. Both
    `--flag value` and `--flag=value` are read."""
    rest, images, i, strict = [], [], 0, False
    while i < len(argv):
        word = argv[i]
        if word == "--strict":
            strict = True
            i += 1
            continue
        flag, eq, inline = word.partition("=")
        if flag in IMAGE_FLAGS or flag in PER_IMAGE_FLAGS:
            if eq:
                value = inline
            else:
                if i + 1 >= len(argv):
                    raise SpecError("FLAG_NEEDS_VALUE", flag)
                i += 1
                value = argv[i]
            if flag in IMAGE_FLAGS:
                images.append(_image_from_flag(flag, value))
            else:
                if not images:
                    raise SpecError("FLAG_BEFORE_IMAGE", flag)
                _apply_flag(images[-1], flag, value)
        else:
            rest.append(word)
        i += 1
    if strict and not images:
        raise SpecError("STRICT_WITHOUT_IMAGES")
    return rest, ({"schema": SCHEMA, "images": images, "strict": True} if images and strict else {"schema": SCHEMA, "images": images} if images else None)


def _image_from_flag(flag, value):
    if flag == "--propimage":               # PATH[:sha256]: any number of prebuilt images (the first brief's spelling)
        m = re.fullmatch(r"(.+):([0-9a-f]{64})", value)
        path, sha = (m.group(1), m.group(2)) if m else (value, None)
        src = parse_source(path)
        if sha:
            src["sha256"] = sha
        return {"kind": "prebuilt", "disk": src}
    kind, sep, src = value.partition(":")
    if not sep or kind not in KINDS or not src:
        raise SpecError("IMAGE_FLAG_BAD", value[:24])
    return {"kind": "prebuilt", "disk": parse_source(src)} if kind == "prebuilt" else {"kind": "built", "iso": parse_source(src)}


def _apply_flag(img, flag, value):
    if flag == "--label":
        img["label"] = value
    elif flag == "--state":
        if value in ("auto", "none"):
            img["state"] = value
        else:
            img["state_file"] = parse_source(value)
    elif flag == "--net":
        img["net"] = value
    elif flag == "--apps":
        img["only"] = [a for a in value.split(",") if a]
    elif flag == "--app":
        name, entry = parse_app(value)
        if name in img.setdefault("apps", {}):
            raise SpecError("APP_DUPLICATE", name[:32])
        img["apps"][name] = entry
    else:
        ssh = img.setdefault("ssh", {})
        key = {"--ssh-user": "user", "--ssh-key": "key_file", "--ssh-port": "port"}[flag]
        if key == "port":
            if not value.isdigit():
                raise SpecError("SSH_BAD", "port")
            ssh[key] = int(value)
        else:
            ssh[key] = value


# ---------------------------------------------------------------- what an image runs
def plan_apps(img, reg, declared=None, probed=None, provided=frozenset()):
    """The per-image application plan: (rows, unsupported installed names, how it was decided).

    rows: [{"app", "action": "run" | "not_run", "reason", "source"}] in registry order, then unknown ids in the order given.
    declared: program names from the image's state.json `software` (None = no state.json or no list);
    probed: installed-program names read from the guest (None = not probed yet).
    Decision order: (1) the apps the spec declares `present`; else (2) the image's own declaration, else (3) the probe, else
    (4) the `only` list taken as the declaration; with none of these `decided` is "NEEDS_PROBE" or "NO_DECLARATION".
    `skip` always wins; `only` always restricts; an id with no definition is NOT RUN NO_DRIVER, never an error."""
    import propimage_registry as R
    explicit = {n: e["mode"] for n, e in img["apps"].items()}
    lanes = {R.canonical(reg, n) or n: e.get("lane", "gate") for n, e in img["apps"].items()}
    skipped = {R.canonical(reg, n) or n for n, m in explicit.items() if m == "skip"}
    present = [R.canonical(reg, n) or n for n, m in explicit.items() if m == "present"]
    unsupported, decided, source = [], "", {}
    if present:
        for n in present:
            source[n] = "spec"
        decided = "SPEC"
    else:
        names = declared if declared is not None else probed
        if names is not None:
            ids, unsupported = R.discover(reg, names)
            for n in ids:
                source[n] = "state" if declared is not None else "probe"
            decided = "STATE" if declared is not None else "PROBE"
        elif img["only"]:
            for n in img["only"]:
                source[R.canonical(reg, n) or n] = "apps-flag"
            decided = "APPS_FLAG"
        else:
            decided = "NEEDS_PROBE"
    only = {R.canonical(reg, n) or n for n in img["only"]}
    wanted = list(source)
    for n in sorted(only - set(wanted)):
        if decided in ("STATE", "PROBE"):
            wanted.append(n)
            source[n] = "apps-flag"
    order = [i for i in reg if i in wanted] + [n for n in wanted if n not in reg]
    rows = []
    for n in order:
        if n in skipped:
            rows.append({"app": n, "action": "not_run", "reason": "SKIPPED", "source": source[n], "lane": lanes.get(n, "gate")})
        elif n not in reg:
            rows.append({"app": n, "action": "not_run", "reason": "NO_DRIVER", "source": source[n], "lane": lanes.get(n, "gate")})
        elif only and n not in only:
            continue                                  # restricted out by `only`: not requested, so not reported
        elif n in only and decided in ("STATE", "PROBE") and source[n] == "apps-flag":
            rows.append({"app": n, "action": "not_run", "reason": "NOT_ON_IMAGE", "source": source[n], "lane": lanes.get(n, "gate")})
        else:
            need = R.unmet_needs(reg[n], provided)
            rows.append({"app": n, "action": "not_run" if need else "run", "reason": ("NEEDS:" + need[0]) if need else "", "source": source[n],
                         "lane": lanes.get(n, "gate")})
    return rows, unsupported, decided


def sha256_file(path, limit=1 << 40):
    h, n = hashlib.sha256(), 0
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 22), b""):
            n += len(chunk)
            if n > limit:
                raise SpecError("FILE_TOO_LARGE")
            h.update(chunk)
    return h.hexdigest()


def describe(spec):
    """One printable line per image (the dry-run / plan output); strictness is on every line."""
    lines = []
    for img in spec["images"]:
        src = img["disk"] or img["iso"]
        lines.append("%s kind=%s from=%s net=%s state=%s strict=%d apps=%s only=%s" % (
            img["label"], img["kind"], src["from"], img["net"], img["state"], int(bool(spec.get("strict"))),
            ",".join("%s:%s" % (n, e["mode"]) for n, e in img["apps"].items()) or "-", ",".join(img["only"]) or "-"))
    return lines


def quote(words):
    return " ".join(shlex.quote(w) for w in words)
