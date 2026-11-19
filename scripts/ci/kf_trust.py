#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""kf_trust: the one place where a repo-defined download gets its trust anchor (stdlib only, works under python3 -I).

STATUS: LIVE, 2026-11-17 (GPU-free; step 1 of docs/design/V3_STANDALONE_TRUST.md; the attestation anchor is a stub that
refuses by name, nothing here ran on a box).

Two actors (docs/OWNER_RULINGS.md section AX). The THIRD PARTY who dislikes opaque blobs gets mode `trust-git-only`
(the default): what the repository downloads by itself must be verified by an anchor that does not depend on us.
The OPERATOR who runs their own Worker, boxes and object store sets mode `strict`, where their pins are deployment data.

Anchors a repo-defined artifact may carry (trust_manifest.json), checked in the order listed:
  official-checksum-url  a checksum document fetched from the UPSTREAM origin (never from a host we run: TRUST_SAME_ORIGIN)
  upstream-pin           a pin in git of an upstream-authored identifier anyone can re-check against upstream: a git commit
                         id, a vendor-published checksum, a registry digest, or the sha256 of a source (or of an upstream
                         artifact upstream still serves byte-identically; that last case is "pending owner confirmation")
  attestation            NOT IMPLEMENTED: refuses by name (TRUST_ATTEST_NOT_IMPLEMENTED), step 2 of the design
A sha256 in git of an artifact WE build or host is not an anchor (class `git-sha256-of-our-own-artifact`, always refused).

A path or URL the USER passes is trusted: anchor `user`, the sha256 and size are recorded, `--expect-sha256` is the user's own
opt-in and is enforced in every mode. In `strict` mode a user object is also checked against the pin the manifest (or the
operator) holds. Lifting the default is explicit and recorded: `--trust-lift REASON` or KF_TRUST_LIFT=REASON turns a
verification failure into anchor `unverified-lifted` (the computed sha256 is recorded; the result is NOT gating). Nothing
implies the lift (not reachability, not a URL scheme, not the absence of a Worker).

Every result is one `trust.jsonl` line (KF_TRUST_EVIDENCE_DIR or --evidence-dir) and one `KF_TRUST ...` line on stderr: the
anchor that verified the artifact, the computed sha256 and size and the origin URL without query, fragment or credentials
(never a presigned URL). Exit status of the CLI: 0 verified, 3 refused (TRUST_<NAME>), 1 transfer failed, 2 usage, 4 no manifest entry for the name.

  kf_trust.py fetch NAME --out PATH [--user-source PATH_OR_URL] [--expect-sha256 HEX] [--mode M] [--trust-lift WHY]
  kf_trust.py verify NAME FILE            same checks on a file already on disk; downloads nothing
  kf_trust.py git-checkout NAME --dest DIR [--user-source REMOTE]
  kf_trust.py git-verify NAME DIR         an existing checkout must be at the pinned commit
  kf_trust.py record --name N --sha256 H --size S [--source LABEL]   an artifact the user supplied that this process did not fetch
  kf_trust.py list | check-evidence FILE
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request

HERE = Path(__file__).resolve().parent
MANIFEST = HERE / "trust_manifest.json"
KF_FETCH = HERE / "kf_fetch.py"

SCHEMA = "kayfabe.trust-manifest/1"
RECORD_SCHEMA = "kayfabe.trust-record/1"

MODE_GIT_ONLY = "trust-git-only"
MODE_STRICT = "strict"
MODES = (MODE_GIT_ONLY, MODE_STRICT)

CLASS_OFFICIAL = "official-checksum-url"
CLASS_PIN = "upstream-pin"
CLASS_ATTEST = "attestation"
CLASS_FORBIDDEN = "git-sha256-of-our-own-artifact"
CLASS_OPERATOR = "operator-pin"  # never in the manifest: it comes from the caller, and only in strict mode
ANCHOR_CLASSES = (CLASS_OFFICIAL, CLASS_PIN, CLASS_ATTEST)

ANCHOR_USER = "user"
ANCHOR_LIFTED = "unverified-lifted"

PIN_ORIGINS = ("vendor-published", "measured-from-upstream", "git-commit", "registry-digest")
SCOPES = ("source", "upstream-artifact")
STATUSES = ("MEASURED", "CARRIED")
KINDS = ("file", "git", "oci-image")
ROUTINGS = ("helper", "native")

# Hosts and GitHub owners whose content is OURS: a checksum document from there is not an upstream checksum.
OUR_HOST_SUFFIXES = ("kayfabe.dev", "r2.dev", "r2.cloudflarestorage.com", "workers.dev")
OUR_GITHUB_OWNERS = ("reindertpelsma",)

# Failures a lift may turn into `unverified-lifted`. A transfer failure and the user's own --expect-sha256 are not on it.
LIFTABLE = ("TRUST_NO_ANCHOR", "TRUST_PIN_MISMATCH", "TRUST_UPSTREAM_SUM_MISMATCH", "TRUST_ATTEST_NOT_IMPLEMENTED",
            "TRUST_ANCHOR_FORBIDDEN", "TRUST_SAME_ORIGIN", "TRUST_UPSTREAM_SUM_BAD_DOC")

SHA = re.compile(r"[0-9a-f]{64}\Z")
COMMIT = re.compile(r"[0-9a-f]{40}\Z")
NAME = re.compile(r"[a-z0-9][a-z0-9._+-]{0,95}\Z")
DATE = re.compile(r"\d{4}-\d{2}-\d{2}\Z")
BLOCK = 1 << 20
DOC_MAX = 1 << 16
DEFAULT_MAX_BYTES = 16 << 30
LIFT_MAX = 200


class TrustError(Exception):
    """A refusal, named: code is TRUST_<NAME>. The text never carries a URL query or a presigned locator."""

    def __init__(self, code, name="", detail=""):
        self.code, self.name, self.detail = code, name, detail
        super().__init__("%s:%s%s" % (code, name, (": " + detail) if detail else ""))


class TransportError(Exception):
    """A transfer that did not complete. kind: NETWORK, SIZE or HASH (the bytes were refused and deleted)."""

    def __init__(self, kind, detail=""):
        self.kind = kind
        super().__init__(kind + (": " + detail if detail else ""))


# ----------------------------------------------------------------------------------------------- small helpers
def sha256_file(path):
    h, n = hashlib.sha256(), 0
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(BLOCK), b""):
            h.update(block)
            n += len(block)
    return h.hexdigest(), n


def glob_escape(text):
    return re.sub(r"([*?\[])", r"[\1]", text)


def redact_url(url):
    """scheme://host[:port]/path: no credentials, query or fragment. Returns (text, redacted?)."""
    p = urllib.parse.urlsplit(url)
    host = p.hostname or ""
    if p.port:
        host += ":%d" % p.port
    return "%s://%s%s" % (p.scheme, host, p.path), bool(p.query or p.fragment or p.username or p.password)


def is_ours(url):
    p = urllib.parse.urlsplit(url)
    host = (p.hostname or "").lower()
    if any(host == s or host.endswith("." + s) for s in OUR_HOST_SUFFIXES):
        return True
    if host in ("github.com", "www.github.com", "raw.githubusercontent.com", "api.github.com"):
        parts = [x for x in p.path.split("/") if x]
        owner = parts[1] if host == "api.github.com" and len(parts) > 1 and parts[0] == "repos" else (parts[0] if parts else "")
        return owner.lower() in OUR_GITHUB_OWNERS
    return False


def valid_https(url, *, allow_loopback_http=False):
    try:
        p = urllib.parse.urlsplit(url)
        loop = allow_loopback_http and p.scheme == "http" and p.hostname in ("127.0.0.1", "localhost", "::1")
        if (p.scheme != "https" and not loop) or not p.hostname or p.username or p.password or p.fragment:
            return False
        return len(url) <= 16384 and not any(ord(c) <= 32 or ord(c) == 127 for c in url)
    except ValueError:
        return False


def check_name(name):
    """An artifact name ends up in log lines and evidence: it is a short word, never text that could forge a line."""
    if not isinstance(name, str) or not NAME.match(name):
        raise TrustError("TRUST_USAGE", "", "an artifact name is 1..96 characters of [a-z0-9._+-], starting with a letter or digit")
    return name


def normalize_mode(mode):
    mode = mode if mode is not None else os.environ.get("KF_TRUST_MODE") or MODE_GIT_ONLY
    if mode not in MODES:
        raise TrustError("TRUST_BAD_MODE", "", "mode must be one of %s" % (MODES,))
    return mode


def normalize_lift(lift):
    """The lift reason, or None. An empty string is no lift; a reason is a short printable text."""
    lift = lift if lift is not None else os.environ.get("KF_TRUST_LIFT")
    if lift is None or lift == "":
        return None
    if not (1 <= len(lift) <= LIFT_MAX) or not lift.isprintable() or lift != lift.strip():
        raise TrustError("TRUST_BAD_LIFT", "", "the lift reason is 1..%d printable characters" % LIFT_MAX)
    return lift


# ------------------------------------------------------------------------------------------------- the manifest
def validate_anchor(a, kind, where):
    """Problems of one anchor, as strings (empty when fine). Used by load_manifest and by trust_lint."""
    bad = []
    if not isinstance(a, dict):
        return [where + ": anchor is not an object"]
    cls = a.get("class")
    if cls == CLASS_FORBIDDEN:
        return [where + ": anchor class %s (a sha256 in git of an artifact we build or host is not an anchor)" % cls]
    if cls not in ANCHOR_CLASSES:
        return [where + ": unknown anchor class %r" % (cls,)]
    if cls == CLASS_OFFICIAL:
        url = a.get("url")
        if kind != "file":
            bad.append(where + ": %s applies to files only" % cls)
        if not isinstance(url, str) or not valid_https(url):
            bad.append(where + ": url must be an https URL")
        elif is_ours(url):
            bad.append(where + ": the checksum URL is one of ours (TRUST_SAME_ORIGIN)")
        if a.get("format") not in ("sha256sum",):
            bad.append(where + ": format must be sha256sum")
    elif cls == CLASS_PIN:
        if a.get("pin_origin") not in PIN_ORIGINS:
            bad.append(where + ": pin_origin must be one of %s" % (PIN_ORIGINS,))
        if a.get("scope") not in SCOPES:
            bad.append(where + ": scope must be one of %s" % (SCOPES,))
        if a.get("status") not in STATUSES:
            bad.append(where + ": status must be one of %s" % (STATUSES,))
        psu = a.get("pin_source_url")
        if not isinstance(psu, str) or not valid_https(psu):
            bad.append(where + ": pin_source_url must be the https URL where anyone re-checks the pin")
        elif is_ours(psu):
            bad.append(where + ": pin_source_url is one of ours: a pin of our own artifact is not an anchor")
        if a.get("status") == "MEASURED" and (not isinstance(a.get("measured"), str) or not DATE.match(a.get("measured", ""))):
            bad.append(where + ": a MEASURED pin carries the YYYY-MM-DD date it was measured")
        if a.get("status") == "CARRIED" and not isinstance(a.get("carried_from"), str):
            bad.append(where + ": a CARRIED pin names the file it was carried from")
        if "measured" in a and not (isinstance(a["measured"], str) and DATE.match(a["measured"])):
            bad.append(where + ": measured must be a YYYY-MM-DD date")
        if kind == "file":
            if not isinstance(a.get("sha256"), str) or not SHA.match(a.get("sha256", "")):
                bad.append(where + ": sha256 must be 64 lowercase hex")
            if type(a.get("size")) is not int or a.get("size", 0) <= 0:
                bad.append(where + ": size must be a positive integer")
        elif kind == "git":
            if not isinstance(a.get("commit"), str) or not COMMIT.match(a.get("commit", "")):
                bad.append(where + ": commit must be 40 lowercase hex")
            if a.get("pin_origin") != "git-commit":
                bad.append(where + ": a git pin has pin_origin git-commit")
        elif kind == "oci-image":
            if not isinstance(a.get("digest"), str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", a.get("digest", "")):
                bad.append(where + ": digest must be sha256:<64 hex>")
            if a.get("pin_origin") != "registry-digest":
                bad.append(where + ": an image pin has pin_origin registry-digest")
    return bad


def confirmation(a):
    """'pending-owner-confirmation' only for the sha256 of an upstream BINARY that no vendor checksum backs; else 'owner-confirmed'."""
    if a.get("class") == CLASS_PIN and a.get("scope") == "upstream-artifact" and a.get("pin_origin") == "measured-from-upstream":
        return "pending-owner-confirmation"
    return "owner-confirmed"


def validate_manifest(doc):
    """All problems of a manifest document, as strings (empty list: valid)."""
    bad = []
    if not isinstance(doc, dict) or doc.get("schema") != SCHEMA:
        return ["schema must be " + SCHEMA]
    arts = doc.get("artifacts")
    if not isinstance(arts, dict) or not arts:
        return ["artifacts must be a non-empty object"]
    for name, e in arts.items():
        w = "artifact %s" % name
        if not NAME.match(name) or not isinstance(e, dict):
            bad.append(w + ": bad name or entry")
            continue
        kind = e.get("kind")
        if kind not in KINDS:
            bad.append(w + ": kind must be one of %s" % (KINDS,))
        if e.get("routing") not in ROUTINGS:
            bad.append(w + ": routing must be one of %s" % (ROUTINGS,))
        row = e.get("row")
        if row is not None and type(row) is not int:
            bad.append(w + ": row is the audit row number or null")
        origin = e.get("origin")
        if kind == "oci-image":
            if not isinstance(origin, str) or not re.fullmatch(r"[a-z0-9./_-]+:[A-Za-z0-9._-]+", origin):
                bad.append(w + ": origin of an image is registry/repo:tag")
        elif not isinstance(origin, str) or not valid_https(origin):
            bad.append(w + ": origin must be an https URL (no credentials)")
        if not isinstance(e.get("consumers"), list) or not e.get("consumers") or not all(isinstance(c, str) for c in e["consumers"]):
            bad.append(w + ": consumers must list the repo files that use the artifact")
        if not isinstance(e.get("runs"), str) or not e.get("runs"):
            bad.append(w + ": runs says where the bytes execute (host, host-root, guest, data, build)")
        anchors = e.get("anchors")
        if not isinstance(anchors, list) or not anchors:
            bad.append(w + ": no anchor")
            continue
        for i, a in enumerate(anchors):
            bad += validate_anchor(a, kind, "%s anchor %d" % (w, i))
        pf = e.get("pin_files", [])
        if not isinstance(pf, list) or not all(isinstance(x, dict) and isinstance(x.get("file"), str) and isinstance(x.get("contains"), list)
                                               and x["contains"] and all(isinstance(c, str) and c for c in x["contains"]) for x in pf):
            bad.append(w + ": pin_files is a list of {file, contains: [strings that file must still hold]}")
        elif e.get("routing") == "native" and not pf:
            bad.append(w + ": native routing names the file that holds the pin (pin_files), so a drift is found")
    return bad


def load_manifest(path=None):
    path = Path(path) if path is not None else MANIFEST
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        raise TrustError("TRUST_BAD_MANIFEST", "", "cannot read %s: %s" % (path.name, type(e).__name__)) from None
    problems = validate_manifest(doc)
    if problems:
        raise TrustError("TRUST_BAD_MANIFEST", "", "; ".join(problems[:3]))
    return doc


def manifest_digest(path=None):
    try:
        return hashlib.sha256(Path(path or MANIFEST).read_bytes()).hexdigest()
    except OSError:
        return None


# --------------------------------------------------------------------------------------------------- transport
class _SameHostHttps(urllib.request.HTTPRedirectHandler):
    """A checksum document may move within its own host, never to another one."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        old, new = urllib.parse.urlsplit(req.full_url), urllib.parse.urlsplit(newurl)
        if new.scheme != "https" or new.hostname != old.hostname:
            return None
        return super().redirect_request(req, fp, code, msg, headers, newurl)


class _HttpsOnly(urllib.request.HTTPRedirectHandler):
    """The bytes are checked against an anchor, so their path may cross hosts, but never leave https."""

    allow_loopback_http = False

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if not valid_https(newurl, allow_loopback_http=self.allow_loopback_http):
            return None
        return super().redirect_request(req, fp, code, msg, headers, newurl)


class HttpTransport:
    """The real transfer layer. Pinned downloads go through scripts/ci/kf_fetch.py (parallel, resumable, fail closed);
    documents and unpinned (user or lifted) downloads use urllib with a byte bound."""

    def __init__(self, allow_loopback_http=False, streams=None, use_kf_fetch=True):
        self.allow_loopback_http = allow_loopback_http
        self.streams = streams if streams is not None else int(os.environ.get("KF_FETCH_STREAMS", "4") or 4)
        self.use_kf_fetch = use_kf_fetch and KF_FETCH.is_file()

    def _check(self, url):
        if not valid_https(url, allow_loopback_http=self.allow_loopback_http):
            raise TransportError("NETWORK", "not an https URL")

    def get_text(self, url, max_bytes=DOC_MAX):
        self._check(url)
        opener = urllib.request.build_opener(_SameHostHttps())
        try:
            with opener.open(urllib.request.Request(url, headers={"Accept-Encoding": "identity"}), timeout=30) as r:
                data = r.read(max_bytes + 1)
        except (OSError, ValueError, urllib.error.URLError) as e:
            getattr(e, "close", lambda: None)()
            raise TransportError("NETWORK", type(e).__name__) from None
        if len(data) > max_bytes:
            raise TransportError("SIZE", "document too large")
        return data

    def download(self, url, dest, *, sha256=None, size=None, max_bytes=DEFAULT_MAX_BYTES):
        """Write the bytes to dest; return (sha256, size). With sha256 given the bytes are verified or TransportError(HASH|SIZE)."""
        self._check(url)
        dest = Path(os.path.abspath(dest))  # kf_fetch runs in a scratch directory: a relative path would mean another file there
        if sha256 is not None and self.use_kf_fetch:
            argv = [sys.executable, "-I", str(KF_FETCH), url, str(dest), "--sha256", sha256, "--streams", str(self.streams),
                    "--follow-redirect", "--state", str(dest) + ".fetch-state.json"]
            if size is not None:
                argv += ["--size", str(size)]
            with tempfile.TemporaryDirectory() as cwd:
                p = subprocess.run(argv, cwd=cwd, capture_output=True, text=True)
            if p.returncode != 0:
                # A failed transfer leaves nothing beside the output path (kf_fetch keeps a partial file and its resume state for the
                # next run of the same download; this layer does not offer that: a later call starts again).
                dest.unlink(missing_ok=True)
                for leftover in dest.parent.glob(glob_escape(dest.name) + ".*"):
                    leftover.unlink(missing_ok=True)
                raise TransportError({3: "HASH", 2: "SIZE"}.get(p.returncode, "NETWORK"), "" if p.returncode in (2, 3) else "kf_fetch exit %d" % p.returncode)
            for leftover in dest.parent.glob(glob_escape(dest.name) + ".fetch-state.json*"):
                leftover.unlink(missing_ok=True)
            return sha256_file(dest)
        handler = _HttpsOnly()
        handler.allow_loopback_http = self.allow_loopback_http
        opener = urllib.request.build_opener(handler)
        limit = size if size is not None else max_bytes
        h, n = hashlib.sha256(), 0
        try:
            with opener.open(urllib.request.Request(url, headers={"Accept-Encoding": "identity"}), timeout=60) as r, open(dest, "wb") as out:
                for block in iter(lambda: r.read(BLOCK), b""):
                    n += len(block)
                    if n > limit:
                        raise TransportError("SIZE", "more than %d bytes" % limit)
                    h.update(block)
                    out.write(block)
        except TransportError:
            dest.unlink(missing_ok=True)
            raise
        except (OSError, ValueError, urllib.error.URLError) as e:
            getattr(e, "close", lambda: None)()
            dest.unlink(missing_ok=True)
            raise TransportError("NETWORK", type(e).__name__) from None
        digest = h.hexdigest()
        if size is not None and n != size:
            dest.unlink(missing_ok=True)
            raise TransportError("SIZE", "%d bytes, expected %d" % (n, size))
        if sha256 is not None and digest != sha256:
            dest.unlink(missing_ok=True)
            raise TransportError("HASH")
        return digest, n


# ------------------------------------------------------------------------------------------------- the anchors
def verify_attestation(name, anchor=None):
    """Step 2 of the design (GitHub attestation, verified offline). Not built: it refuses, by name, never passes."""
    raise TrustError("TRUST_ATTEST_NOT_IMPLEMENTED", name, "the attestation anchor is not implemented in this tree")


def parse_sha256sum(doc, member=None):
    """The sha256 for `member` from a `<hex>  <name>` document (or the only hash in it when no member is named); else None."""
    rows = []
    for line in doc.decode("ascii", "replace").splitlines():
        m = re.fullmatch(r"([0-9a-fA-F]{64})\s+\*?(\S.*?)\s*", line)
        if m:
            rows.append((m.group(1).lower(), m.group(2)))
    if member is None:
        return rows[0][0] if len(rows) == 1 else None
    found = [h for h, n in rows if n == member or n.rsplit("/", 1)[-1] == member]
    return found[0] if len(found) == 1 else None


def _expectations(name, entry, mode, operator_pin, transport):
    """Evaluate every anchor of a file artifact that can give an expected hash. Returns (expectations, skipped).
    expectation = (anchor_class, sha256, size|None, detail); skipped = [{anchor, code}] for anchors that could not be evaluated."""
    exp, skipped = [], []
    for a in entry.get("anchors", []):
        cls = a.get("class")
        if cls == CLASS_FORBIDDEN:
            raise TrustError("TRUST_ANCHOR_FORBIDDEN", name, "a sha256 in git of an artifact we build or host is not an anchor")
        if cls == CLASS_ATTEST:
            try:
                verify_attestation(name, a)
            except TrustError as e:
                skipped.append({"anchor": CLASS_ATTEST, "code": e.code})
            continue
        if cls == CLASS_OFFICIAL:
            url = a.get("url", "")
            if is_ours(url):
                raise TrustError("TRUST_SAME_ORIGIN", name, "the checksum would come from a host we run")
            try:
                doc = transport.get_text(url)
            except TransportError as e:
                skipped.append({"anchor": CLASS_OFFICIAL, "code": "TRUST_TRANSPORT", "kind": e.kind})
                continue
            h = parse_sha256sum(doc, a.get("member"))
            if h is None:
                raise TrustError("TRUST_UPSTREAM_SUM_BAD_DOC", name, "the upstream checksum document has no single hash for the artifact")
            exp.append((CLASS_OFFICIAL, h, None, {"checksum_url": redact_url(url)[0]}))
        elif cls == CLASS_PIN:
            exp.append((CLASS_PIN, a["sha256"], a.get("size"), {"pin_origin": a.get("pin_origin"), "scope": a.get("scope"),
                                                                 "pin_source_url": a.get("pin_source_url"), "status": a.get("status"),
                                                                 "measured": a.get("measured"),
                                                                 "confirmation": confirmation(a)}))
        else:
            raise TrustError("TRUST_BAD_MANIFEST", name, "unknown anchor class")
    if mode == MODE_STRICT and operator_pin:
        exp.append((CLASS_OPERATOR, operator_pin, None, {"deployment_data": True}))
    return exp, skipped


def _pin_state(entry, sha, size):
    """How a user object compares with the repository's pin: match, differs or none (informational unless strict)."""
    for a in entry.get("anchors", []):
        if a.get("class") == CLASS_PIN and a.get("sha256"):
            return "match" if (a["sha256"] == sha and a.get("size") in (None, size)) else "differs"
    return "none"


# -------------------------------------------------------------------------------------------------- the result
def make_result(name, *, path, sha256, size, anchor, mode, origin=None, source_kind="origin", detail=None, gating=True, lift_reason=None,
                commit=None, redacted=False):
    r = {"schema": RECORD_SCHEMA, "name": name, "anchor": anchor, "sha256": sha256, "size": size, "origin": origin,
         "origin_redacted": redacted, "source": source_kind, "mode": mode, "gating": gating, "detail": detail or {},
         "path": str(path) if path is not None else None}
    if commit is not None:
        r["commit"] = commit
    if lift_reason is not None:
        r["lift_reason"] = lift_reason
    return r


def record(result, evidence_dir=None, clock=None, out=None):
    """Append the result to <evidence_dir>/trust.jsonl (when a directory is given or KF_TRUST_EVIDENCE_DIR is set) and print it."""
    out = out or sys.stderr  # stdout belongs to the caller's own contract (qemu_source.py prints the verified path there)
    line = dict(result)
    line["at"] = (clock() if clock else datetime.datetime.now(datetime.timezone.utc)).strftime("%Y-%m-%dT%H:%M:%SZ")
    line["manifest_sha256"] = manifest_digest()
    line.pop("path", None)
    d = evidence_dir or os.environ.get("KF_TRUST_EVIDENCE_DIR")
    if d:
        Path(d).mkdir(parents=True, exist_ok=True)
        fd = os.open(str(Path(d) / "trust.jsonl"), os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o644)
        try:
            os.write(fd, (json.dumps(line, sort_keys=True) + "\n").encode())
        finally:
            os.close(fd)
    print("KF_TRUST name=%s anchor=%s sha256=%s size=%s mode=%s gating=%s" % (
        result["name"], result["anchor"], result["sha256"] or "-", result["size"] if result["size"] is not None else "-",
        result["mode"], "yes" if result["gating"] else "no"), file=out, flush=True)
    if result["anchor"] == ANCHOR_LIFTED:
        print("KF_TRUST_LIFTED name=%s reason=%s lifted=%s" % (result["name"], result.get("lift_reason"),
                                                               result["detail"].get("lifted_error")), file=out, flush=True)
    return line


# ------------------------------------------------------------------------------------------------------ resolve
def _publish(tmp, final):
    """Move verified bytes into place with the mode a plain download would have (kf_fetch writes 0600; curl and urllib follow the umask)."""
    mask = os.umask(0)
    os.umask(mask)
    os.chmod(tmp, 0o666 & ~mask)
    os.replace(tmp, final)


def _discard(*paths):
    for p in paths:
        try:
            os.unlink(p)
        except FileNotFoundError:
            pass


def _user_object(name, entry, out, user_source, expect, transport, mode, operator_pin):
    """The user's own object: trusted, hashed, recorded; --expect-sha256 enforced; the repository pin enforced in strict mode only."""
    is_url = re.match(r"[a-zA-Z][a-zA-Z0-9+.-]*://", user_source) is not None
    if is_url:
        if out is None:
            raise TrustError("TRUST_USAGE", name, "a URL needs --out")
        tmp = Path(str(out) + ".unverified")
        _discard(tmp)
        try:
            sha, size = transport.download(user_source, tmp, max_bytes=DEFAULT_MAX_BYTES)
        except TransportError as e:
            _discard(tmp)
            raise TrustError("TRUST_TRANSPORT", name, e.kind) from None
        origin, redacted = redact_url(user_source)
        kind, final = "user-url", Path(out)
    else:
        src = Path(user_source)
        if not src.is_file():
            raise TrustError("TRUST_TRANSPORT", name, "the user path is not a file")
        sha, size = sha256_file(src)
        origin, redacted, kind = "file:" + src.name, False, "user-path"
        tmp, final = None, (Path(out) if out is not None else src)
    refusal = None
    if expect and sha != expect:
        refusal = TrustError("TRUST_EXPECT_MISMATCH", name, "computed %s, --expect-sha256 says %s" % (sha, expect))
    state = _pin_state(entry, sha, size)
    if refusal is None and mode == MODE_STRICT:
        if state == "differs":
            refusal = TrustError("TRUST_PIN_MISMATCH", name, "strict mode: the object is not the pinned one (computed %s)" % sha)
        elif operator_pin and operator_pin != sha:
            refusal = TrustError("TRUST_PIN_MISMATCH", name, "strict mode: the object is not the operator's pin (computed %s)" % sha)
    if refusal is not None:
        if tmp is not None:
            _discard(tmp)
        raise refusal
    if tmp is not None:
        _publish(tmp, final)
    elif Path(out or src).resolve() != src.resolve():
        part = Path(str(final) + ".unverified")
        shutil.copyfile(src, part)
        _publish(part, final)
    return make_result(name, path=final, sha256=sha, size=size, anchor=ANCHOR_USER, mode=mode, origin=origin, source_kind=kind,
                       redacted=redacted, detail={"pin": state, "pin_enforced": mode == MODE_STRICT,
                                                  "expect_sha256": bool(expect)})


def resolve(name, out=None, *, manifest=None, user_source=None, expect_sha256=None, mode=None, lift=None, operator_pin=None,
            transport=None, evidence_dir=None, download=True, clock=None, emit=True):
    """Obtain (or check) the file artifact `name` and return its result dict; raise TrustError when no anchor verifies it.

    out          where the verified bytes end up; they appear there only after every check passed
    user_source  a path or URL the user passes: trusted and recorded (anchor `user`)
    expect_sha256  the user's own opt-in; enforced in every mode and never liftable
    mode         trust-git-only (default, KF_TRUST_MODE) or strict (the operator's mode)
    lift         the explicit reason that turns a verification failure into `unverified-lifted`
    operator_pin the operator's own sha256 (deployment data); an anchor only in strict mode
    download     False: verify what is at `out` and never touch the network for the artifact
    """
    check_name(name)
    mode, lift = normalize_mode(mode), normalize_lift(lift)
    if expect_sha256 is not None and not SHA.match(expect_sha256):
        raise TrustError("TRUST_USAGE", name, "--expect-sha256 is 64 lowercase hex")
    if operator_pin is not None and not SHA.match(operator_pin):
        raise TrustError("TRUST_USAGE", name, "the operator pin is 64 lowercase hex")
    manifest = manifest if manifest is not None else load_manifest()
    entry = (manifest.get("artifacts") or {}).get(name)
    if entry is None and user_source is not None:
        entry = {"kind": "file", "anchors": []}  # the user's own object needs no entry: it is trusted, hashed and recorded
    if entry is None:
        raise TrustError("TRUST_UNKNOWN_ARTIFACT", name, "not in the trust manifest")
    if entry.get("kind") != "file":
        raise TrustError("TRUST_USAGE", name, "not a file artifact (kind %s)" % entry.get("kind"))
    transport = transport or HttpTransport()
    out = Path(os.path.abspath(out)) if out is not None else None
    if user_source is not None:
        result = _user_object(name, entry, out, user_source, expect_sha256, transport, mode, operator_pin)
    else:
        if out is None:
            raise TrustError("TRUST_USAGE", name, "--out is required")
        try:
            result = _anchored(name, entry, out, expect_sha256, mode, operator_pin, transport, download)
        except TrustError as e:
            if lift is None or e.code not in LIFTABLE or not download:
                raise
            result = _lifted(name, entry, out, mode, lift, e, transport)
    if emit:
        record(result, evidence_dir, clock)
    return result


def _anchored(name, entry, out, expect, mode, operator_pin, transport, download):
    exp, skipped = _expectations(name, entry, mode, operator_pin, transport)
    if not exp:
        code = skipped[-1]["code"] if skipped else "TRUST_NO_ANCHOR"
        raise TrustError(code, name, "no anchor of this artifact could be evaluated (%s)" % ", ".join(
            "%s: %s" % (s["anchor"], s["code"]) for s in skipped) if skipped else "the manifest lists no anchor")
    hashes = {e[1] for e in exp}
    if len(hashes) != 1:
        raise TrustError("TRUST_PIN_MISMATCH", name, "the anchors disagree: " + ", ".join("%s says %s" % (e[0], e[1]) for e in exp))
    want = exp[0][1]
    size = next((e[2] for e in exp if e[2] is not None), None)
    if expect and expect != want:
        raise TrustError("TRUST_EXPECT_MISMATCH", name, "--expect-sha256 is not what the anchors say (%s)" % want)
    cache = False
    if out.is_file():
        have, n = sha256_file(out)
        cache = have == want and size in (None, n)
        if not cache and not download:
            raise TrustError(_mismatch_code(exp), name, "the file at the output path is not the anchored object (computed %s)" % have)
    elif not download:
        raise TrustError("TRUST_TRANSPORT", name, "no file at the output path")
    if cache:
        got, n = want, out.stat().st_size
    else:
        tmp = Path(str(out) + ".unverified")
        _discard(tmp, out)
        origin = entry["origin"]
        try:
            got, n = transport.download(origin, tmp, sha256=want, size=size)
        except TransportError as e:
            _discard(tmp)
            if e.kind in ("HASH", "SIZE"):
                raise TrustError(_mismatch_code(exp), name, "the download is not the anchored object (%s)" % e.kind) from None
            raise TrustError("TRUST_TRANSPORT", name, e.kind) from None
        # The transport is not trusted to have enforced the anchor: compare what it reports.
        if got != want or (size is not None and n != size):
            _discard(tmp)
            raise TrustError(_mismatch_code(exp), name, "the download is not the anchored object (computed %s)" % got)
        _publish(tmp, out)
    first = exp[0]
    detail = dict(first[3])
    detail["also_verified"] = [e[0] for e in exp[1:]]
    if skipped:
        detail["skipped"] = skipped
    if cache:
        detail["cache_hit"] = True
    origin, redacted = redact_url(entry["origin"])
    return make_result(name, path=out, sha256=got, size=n, anchor=first[0], mode=mode, origin=origin, redacted=redacted,
                       source_kind="origin", detail=detail)


def _mismatch_code(exp):
    return "TRUST_UPSTREAM_SUM_MISMATCH" if any(e[0] == CLASS_OFFICIAL for e in exp) else "TRUST_PIN_MISMATCH"


def _lifted(name, entry, out, mode, lift, error, transport):
    tmp = Path(str(out) + ".unverified")
    _discard(tmp)
    sizes = [a.get("size") for a in entry.get("anchors", []) if type(a.get("size")) is int]
    try:
        got, n = transport.download(entry["origin"], tmp, max_bytes=max(sizes) * 2 if sizes else DEFAULT_MAX_BYTES)
    except TransportError as e:
        _discard(tmp)
        raise TrustError("TRUST_TRANSPORT", name, e.kind) from None
    _publish(tmp, out)
    origin, redacted = redact_url(entry["origin"])
    return make_result(name, path=out, sha256=got, size=n, anchor=ANCHOR_LIFTED, mode=mode, origin=origin, redacted=redacted,
                       detail={"lifted_error": error.code}, gating=False, lift_reason=lift)


# ------------------------------------------------------------------------------------------------------- git
def _git(runner, argv, cwd=None, env=None):
    e = {**os.environ, "GIT_TERMINAL_PROMPT": "0", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}
    if env:
        e.update(env)
    return runner(["git", *argv], cwd=cwd, env=e, capture_output=True, text=True)


def resolve_git(name, dest, *, manifest=None, user_source=None, mode=None, lift=None, runner=subprocess.run, evidence_dir=None,
                clock=None, emit=True, allow_local=False):
    """Check out the pinned commit of a git artifact into dest, or fail: HEAD must equal the pinned commit (a tag can move).
    A user remote (user_source) is trusted: whatever it holds at the anchor's ref is checked out and its commit is recorded."""
    check_name(name)
    mode, lift = normalize_mode(mode), normalize_lift(lift)
    manifest = manifest if manifest is not None else load_manifest()
    entry = (manifest.get("artifacts") or {}).get(name)
    if entry is None:
        raise TrustError("TRUST_UNKNOWN_ARTIFACT", name, "not in the trust manifest")
    if entry.get("kind") != "git":
        raise TrustError("TRUST_USAGE", name, "not a git artifact (kind %s)" % entry.get("kind"))
    pin = next((a for a in entry.get("anchors", []) if a.get("class") == CLASS_PIN and a.get("commit")), None)
    ref = (pin or {}).get("ref") or ""
    remote = user_source if user_source is not None else entry["origin"]
    if user_source is None and not allow_local and not valid_https(remote):
        raise TrustError("TRUST_USAGE", name, "origin is not https")
    dest = Path(os.path.abspath(dest))
    shutil.rmtree(dest, ignore_errors=True)
    dest.mkdir(parents=True)

    def fail(code, detail):
        shutil.rmtree(dest, ignore_errors=True)
        return TrustError(code, name, detail)

    def git(*argv):
        return _git(runner, ["-C", str(dest), *argv])

    if git("init", "-q").returncode != 0 or git("remote", "add", "origin", remote).returncode != 0:
        raise fail("TRUST_TRANSPORT", "git setup failed")
    # What to fetch: the pinned commit by id (the id is the identity), else the tag as a hint for servers that refuse an id.
    attempts = []
    if pin is not None and user_source is None:
        attempts.append((pin["commit"], "FETCH_HEAD"))
    if ref:
        attempts.append(("refs/tags/%s:refs/tags/%s" % (ref, ref), "refs/tags/" + ref))
    if user_source is not None and not ref:
        attempts.append(("HEAD", "FETCH_HEAD"))
    target = None
    for spec, tgt in attempts:
        if git("fetch", "-q", "--depth", "1", "origin", spec).returncode == 0:
            target = tgt
            break
    if target is None:
        raise fail("TRUST_TRANSPORT", "git fetch failed")
    if git("checkout", "-q", "--detach", target).returncode != 0:
        raise fail("TRUST_TRANSPORT", "git checkout failed")
    head = git("rev-parse", "HEAD").stdout.strip()
    if not COMMIT.match(head):
        raise fail("TRUST_TRANSPORT", "git rev-parse gave no commit")
    origin, redacted = redact_url(remote) if re.match(r"[a-z]+://", remote) else ("file:" + Path(remote).name, False)
    base = dict(path=dest, sha256=None, size=None, mode=mode, origin=origin, redacted=redacted, commit=head)
    if user_source is not None:
        state = "none" if pin is None else ("match" if pin["commit"] == head else "differs")
        if mode == MODE_STRICT and state == "differs":
            raise fail("TRUST_PIN_MISMATCH", "strict mode: the checked-out commit %s is not the pinned one" % head)
        result = make_result(name, anchor=ANCHOR_USER, source_kind="user-url", detail={"ref": ref, "pin": state,
                                                                                         "pin_enforced": mode == MODE_STRICT}, **base)
    else:
        problem = None
        if pin is None:
            problem = ("TRUST_NO_ANCHOR", "the manifest lists no commit pin")
        elif head != pin["commit"]:
            problem = ("TRUST_PIN_MISMATCH", "HEAD %s is not the pinned commit %s" % (head, pin["commit"]))
        if problem and lift is None:
            raise fail(*problem)
        if problem:
            result = make_result(name, anchor=ANCHOR_LIFTED, gating=False, lift_reason=lift,
                                 detail={"lifted_error": problem[0]}, **base)
        else:
            result = make_result(name, anchor=CLASS_PIN, detail={"pin_origin": pin.get("pin_origin"), "scope": pin.get("scope"),
                                                                 "ref": ref, "pin_source_url": pin.get("pin_source_url"),
                                                                 "status": pin.get("status"), "measured": pin.get("measured"),
                                                                 "confirmation": confirmation(pin)}, **base)
    if emit:
        record(result, evidence_dir, clock)
    return result


def verify_git(name, dest, *, manifest=None, mode=None, lift=None, runner=subprocess.run, evidence_dir=None, clock=None, emit=True):
    """Check that an existing checkout is at the pinned commit (a cached source tree). Raises TRUST_PIN_MISMATCH when it is not."""
    check_name(name)
    mode, lift = normalize_mode(mode), normalize_lift(lift)
    manifest = manifest if manifest is not None else load_manifest()
    entry = (manifest.get("artifacts") or {}).get(name)
    if entry is None:
        raise TrustError("TRUST_UNKNOWN_ARTIFACT", name, "not in the trust manifest")
    pin = next((a for a in entry.get("anchors", []) if a.get("class") == CLASS_PIN and a.get("commit")), None)
    p = _git(runner, ["-C", str(dest), "rev-parse", "HEAD"])
    head = p.stdout.strip() if p.returncode == 0 else ""
    if not COMMIT.match(head):
        raise TrustError("TRUST_TRANSPORT", name, "not a git checkout")
    if pin is None or head != pin["commit"]:
        err = TrustError("TRUST_NO_ANCHOR" if pin is None else "TRUST_PIN_MISMATCH", name, "HEAD %s is not the pinned commit" % head)
        if lift is None:
            raise err
        anchor, gating, detail = ANCHOR_LIFTED, False, {"lifted_error": err.code}
    else:
        anchor, gating, detail = CLASS_PIN, True, {"pin_origin": pin.get("pin_origin"), "pin_source_url": pin.get("pin_source_url"),
                                                   "status": pin.get("status"), "confirmation": confirmation(pin), "cache_hit": True}
    result = make_result(name, path=dest, sha256=None, size=None, anchor=anchor, mode=mode, origin=redact_url(entry["origin"])[0], commit=head,
                         detail=detail, gating=gating, lift_reason=lift if anchor == ANCHOR_LIFTED else None)
    if emit:
        record(result, evidence_dir, clock)
    return result


# --------------------------------------------------------------------------------------------------------- CLI
# CLI exit status by refusal: a caller may fall back on 1 (transfer) or 4 (no entry for this version); 3 is a refusal to keep.
EXIT = {"TRUST_TRANSPORT": 1, "TRUST_USAGE": 2, "TRUST_BAD_MODE": 2, "TRUST_BAD_LIFT": 2, "TRUST_BAD_MANIFEST": 2, "TRUST_UNKNOWN_ARTIFACT": 4}


def _common(p):
    p.add_argument("--manifest", default=None, help="trust manifest (tests only)")
    p.add_argument("--mode", default=None, choices=MODES, help="trust-git-only (default) or strict (the operator's mode); KF_TRUST_MODE")
    p.add_argument("--trust-lift", default=None, metavar="REASON", help="explicit, recorded: failures become unverified-lifted; KF_TRUST_LIFT")
    p.add_argument("--evidence-dir", default=None, help="directory of trust.jsonl; KF_TRUST_EVIDENCE_DIR")


class _Parser(argparse.ArgumentParser):
    def error(self, message):
        self.exit(2, "KF_TRUST_USAGE\n")


def main(argv=None):
    ap = _Parser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("fetch")
    f.add_argument("name")
    f.add_argument("--out", required=True)
    f.add_argument("--user-source")
    f.add_argument("--expect-sha256")
    f.add_argument("--operator-pin")
    _common(f)
    v = sub.add_parser("verify")
    v.add_argument("name")
    v.add_argument("file")
    v.add_argument("--expect-sha256")
    v.add_argument("--operator-pin")
    _common(v)
    g = sub.add_parser("git-checkout")
    g.add_argument("name")
    g.add_argument("--dest", required=True)
    g.add_argument("--user-source")
    _common(g)
    gv = sub.add_parser("git-verify")
    gv.add_argument("name")
    gv.add_argument("dir")
    _common(gv)
    r = sub.add_parser("record")
    r.add_argument("--name", required=True)
    r.add_argument("--sha256", required=True)
    r.add_argument("--size", type=int, required=True)
    r.add_argument("--source", default="user")
    r.add_argument("--origin")
    _common(r)
    sub.add_parser("list").add_argument("--manifest", default=None)
    c = sub.add_parser("check-evidence")
    c.add_argument("file")
    a = ap.parse_args(argv)
    try:
        if a.cmd == "list":
            for n, e in sorted(load_manifest(a.manifest)["artifacts"].items()):
                print("%s\t%s\t%s" % (n, e["kind"], ",".join(x["class"] for x in e["anchors"])))
        elif a.cmd == "check-evidence":
            return check_evidence(a.file)
        elif a.cmd == "record":
            check_name(a.name)
            mode, lift = normalize_mode(a.mode), normalize_lift(a.trust_lift)
            if not SHA.match(a.sha256):
                raise TrustError("TRUST_USAGE", a.name, "sha256 is 64 lowercase hex")
            origin, red = redact_url(a.origin) if a.origin else (None, False)
            record(make_result(a.name, path=None, sha256=a.sha256, size=a.size, anchor=ANCHOR_USER, mode=mode, origin=origin,
                               source_kind=a.source, redacted=red, detail={"recorded_only": True}), a.evidence_dir)
        elif a.cmd == "git-verify":
            verify_git(a.name, a.dir, manifest=load_manifest(a.manifest) if a.manifest else None, mode=a.mode, lift=a.trust_lift,
                       evidence_dir=a.evidence_dir)
        elif a.cmd == "git-checkout":
            resolve_git(a.name, a.dest, manifest=load_manifest(a.manifest) if a.manifest else None, user_source=a.user_source,
                        mode=a.mode, lift=a.trust_lift, evidence_dir=a.evidence_dir)
        else:
            man = load_manifest(a.manifest) if a.manifest else None
            if a.cmd == "verify":
                resolve(a.name, a.file, manifest=man, expect_sha256=a.expect_sha256, mode=a.mode, lift=a.trust_lift,
                        operator_pin=a.operator_pin, evidence_dir=a.evidence_dir, download=False)
            else:
                resolve(a.name, a.out, manifest=man, user_source=a.user_source, expect_sha256=a.expect_sha256, mode=a.mode,
                        lift=a.trust_lift, operator_pin=a.operator_pin, evidence_dir=a.evidence_dir)
    except TrustError as e:
        print("%s" % e, file=sys.stderr)
        return EXIT.get(e.code, 3)
    return 0


def check_evidence(path):
    """The run report's rule: a result with any `unverified-lifted` line is not gating (exit 3); a malformed line is exit 2."""
    lifted = 0
    try:
        for n, line in enumerate(Path(path).read_text(encoding="utf-8").splitlines(), 1):
            if not line.strip():
                continue
            row = json.loads(line)
            if row.get("schema") != RECORD_SCHEMA or row.get("anchor") not in (ANCHOR_USER, ANCHOR_LIFTED, CLASS_OFFICIAL, CLASS_PIN, CLASS_OPERATOR):
                print("KF_TRUST_EVIDENCE_BAD line=%d" % n, file=sys.stderr)
                return 2
            if row["anchor"] == ANCHOR_LIFTED or row.get("gating") is False:
                lifted += 1
    except (OSError, ValueError):
        print("KF_TRUST_EVIDENCE_BAD", file=sys.stderr)
        return 2
    if lifted:
        print("KF_TRUST_NOT_GATING lifted=%d" % lifted, file=sys.stderr)
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
