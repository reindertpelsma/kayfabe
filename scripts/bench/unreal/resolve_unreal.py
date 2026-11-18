# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Build-or-fetch: decide where the Unreal installed build for a run comes from (stdlib only, no network, no process started).

STATUS: DESIGN-ONLY tooling, 2026-11-17 (GPU-free unit tests; nothing was fetched or built by it). Design:
docs/design/V3_UNREAL_IMAGE.md section 4.

Order, first match wins:
  1. an explicit artifact (`--unreal-artifact PATH|URL`, with its manifest): use it. Its members are checked against its own
     manifest in every mode. With a source also given, the key the source and recipe produce must equal the manifest's key.
  2. a cache entry named by the deployment (instruction or URL file): fetch it. Strict and worker runs enforce the entry's
     size and sha256 (the deployment's pin); a standalone run records the sha256 it computed and enforces nothing from it.
  3. a source (`--unreal-source PATH`, or `--unreal-source-clone`): build it (hours; says so) and optionally write the artifact
     and its manifest to `--unreal-cache-out DIR`. Never in worker mode: boxes are untrusted and never build.
  4. nothing: standalone SKIPS (UNREAL_NO_SOURCE_OR_CACHE, not a failure); strict FAILS with the same class; worker FAILS with
     UNREAL_CACHE_MISS (rebuild on the trusted machine, publish, pin).

Modes: standalone (the default for a user's own machine), strict (a user who wants a missing input to fail), worker (a rented box
under the deployment's Worker; pins enforced; no build).
"""

import os
import re

try:
    from . import ue_lib as L
except ImportError:  # run as a script or from the tests with the directory on sys.path
    import ue_lib as L

URL_RE = re.compile(r"^https?://", re.I)


class Opts:
    def __init__(self, mode="standalone", artifact=None, manifest=None, source=None, source_clone=None, source_ref="release",
                 cache_listing=None, expect_sha256=None, cache_out=None, source_id=None, allow_dirty=False):
        self.mode, self.artifact, self.manifest = mode, artifact, manifest
        self.source, self.source_clone, self.source_ref = source, source_clone, source_ref
        self.cache_listing, self.expect_sha256, self.cache_out = cache_listing, expect_sha256, cache_out
        self.source_id, self.allow_dirty = source_id, allow_dirty


class Facts:
    """What the decision may ask about the world. The tests pass a fake with the same methods."""

    def compute_key(self, source, source_id, allow_dirty):
        return L.compute_key(L.Runner(), source, allow_dirty, source_id)

    def load_manifest(self, path):
        return L.load_manifest(path)

    def member_check(self, path, manifest):
        return L.check_member(path, manifest)

    def file_sha256(self, path):
        return L.sha256_file(path)

    def gh_access(self, repo):
        """(ok, reason). The real implementation lives in build_installed.py (`gh auth status`, then one repository read);
        it never prompts and never prints a token."""
        return False, "gh_not_logged_in"

    def isdir(self, p):
        return os.path.isdir(p)

    def isfile(self, p):
        return os.path.isfile(p)


class Decision:
    def __init__(self, action, cls="", detail="", key=None, **extra):
        self.action, self.cls, self.detail, self.key = action, cls, detail, key
        self.extra = extra

    @property
    def ok(self):
        return self.action in ("use_artifact", "use_tree", "fetch", "build") or (self.action == "skip")

    @property
    def exit_code(self):
        return 0 if self.ok else 3

    def as_dict(self):
        d = {"action": self.action, "class": self.cls, "detail": self.detail, "key": self.key}
        d.update(self.extra)
        return d

    def line(self):
        return "KF_UNREAL_DECISION action=%s class=%s key=%s detail=%s" % (self.action, self.cls or "-", (self.key or "-")[:12], self.detail.replace("\n", " ")[:200] or "-")


def _fail(cls, detail="", key=None, **extra):
    return Decision("fail", cls, detail, key, **extra)


def _manifest_path_for(artifact, explicit):
    return explicit or os.path.join(os.path.dirname(os.path.abspath(artifact)), "manifest.json")


def _pin_problem(sha, size, pin):
    if pin.get("sha256") and sha != pin["sha256"]:
        return "sha256 %s differs from the deployment's pin %s" % (sha[:12], str(pin["sha256"])[:12])
    if pin.get("size") is not None and size != pin["size"]:
        return "size %d differs from the deployment's pin %d" % (size, pin["size"])
    return ""


def decide(o, facts=None):
    f = facts or Facts()
    if o.mode not in L.MODES:
        return _fail(L.FAIL_USAGE, "mode must be one of %s" % ", ".join(L.MODES))
    enforce_pins = o.mode in ("strict", "worker")
    if o.expect_sha256 is not None and not L.RE_HEX64.match(o.expect_sha256):
        return _fail(L.FAIL_USAGE, "--expect-sha256 is not 64 lowercase hex")
    if o.mode == "worker" and (o.source or o.source_clone):
        return _fail(L.FAIL_WORKER_BUILD, "a rented box never builds an artifact; rebuild on the trusted machine and publish it")
    if o.source and o.source_clone:
        return _fail(L.FAIL_USAGE, "give --unreal-source or --unreal-source-clone, not both")

    # 1. an explicit artifact
    if o.artifact:
        return _use_artifact(o, f, enforce_pins)

    # 2. a cache entry the deployment names
    listing = o.cache_listing
    if listing:
        key = listing.get("key")
        if not (isinstance(key, str) and L.RE_HEX64.match(key)):
            return _fail(L.FAIL_MANIFEST, "the cache entry carries no key")
        if o.source:
            try:
                comp = f.compute_key(o.source, o.source_id, o.allow_dirty)
            except L.UnrealError as e:
                return _fail(e.cls, e.detail)
            if comp["key"] != key:
                if o.mode == "standalone":
                    return _build(o, f, comp, note="UNREAL_CACHE_KEY_MISMATCH action=rebuild listed=%s computed=%s" % (key[:12], comp["key"][:12]))
                return _fail(L.FAIL_KEY_MISMATCH, "the listed cache is for key %s, this source and recipe produce %s" % (key[:12], comp["key"][:12]), comp["key"])
        return Decision("fetch", "", "cache entry %s" % key[:12], key, enforce_pin=listing if enforce_pins else None,
                        record_sha256=True, expect_sha256=o.expect_sha256, verify_after=["member", "key"] + (["pin"] if enforce_pins else []))

    # 3. a source: build it
    if o.source or o.source_clone:
        if o.source:
            try:
                comp = f.compute_key(o.source, o.source_id, o.allow_dirty)
            except L.UnrealError as e:
                return _fail(e.cls, e.detail)
            return _build(o, f, comp)
        ok, why = f.gh_access(o.source_clone)
        if not ok:
            if o.mode == "standalone":
                return Decision("skip", L.SKIP_NO_SOURCE_OR_CACHE, "cannot clone %s: %s (not a failure)" % (o.source_clone, why))
            return _fail(L.FAIL_CLONE, "cannot clone %s: %s" % (o.source_clone, why))
        return _build(o, f, None, clone=True)

    # 4. nothing
    if o.mode == "worker":
        return _fail(L.FAIL_CACHE_MISS, "no artifact for this run; rebuild on the trusted machine with build_installed.py, upload it, and pin it in the deployment's config")
    if o.mode == "strict":
        return _fail(L.SKIP_NO_SOURCE_OR_CACHE, "no source and no artifact (strict: a missing input fails)")
    return Decision("skip", L.SKIP_NO_SOURCE_OR_CACHE, "no --unreal-artifact, no cache entry and no --unreal-source: the Unreal rows are skipped (not a failure)")


def _build(o, f, comp, clone=False, note=""):
    phases = L.phases_for(clone, bool(o.cache_out))
    return Decision("build", "", "hours: %s" % ("clone, " if clone else "") + "setup, build" + (", package" if o.cache_out else ""), comp["key"] if comp else None,
                    phases=phases, plan=L.plan_text(phases), cache_out=o.cache_out, clone=o.source_clone if clone else None,
                    ref=o.source_ref if clone else None, note=note, record_sha256=True)


def _use_artifact(o, f, enforce_pins):
    a = o.artifact
    if URL_RE.match(a):
        if not o.manifest:
            return _fail(L.FAIL_MANIFEST, "an artifact URL needs --unreal-manifest (a path or a URL)")
        return Decision("fetch", "", "artifact URL", None, record_sha256=True, expect_sha256=o.expect_sha256,
                        verify_after=["member", "key"] + (["pin"] if enforce_pins else []),
                        enforce_pin=o.cache_listing if enforce_pins else None)
    if f.isdir(a):
        if enforce_pins:
            return _fail(L.FAIL_MANIFEST, "a directory has no manifest and no key; strict and worker runs need a manifest")
        problems, _w = L.verify_tree(a)
        if problems:
            return _fail(L.FAIL_TREE, "not an installed build: missing %s" % ", ".join(problems))
        return Decision("use_tree", "", "installed-build directory supplied by the user (no manifest: provenance is theirs)", None,
                        key_verified=False, record_sha256=False)
    if not f.isfile(a):
        return _fail(L.FAIL_USAGE, "no such artifact: %s" % os.path.basename(a))
    mp = _manifest_path_for(a, o.manifest)
    if not f.isfile(mp):
        return _fail(L.FAIL_MANIFEST, "no manifest next to the artifact (looked for %s)" % os.path.basename(mp))
    try:
        man = f.load_manifest(mp)
        sha, size = f.member_check(a, man)  # integrity against its OWN manifest: every mode
    except L.UnrealError as e:
        return _fail(e.cls, e.detail)
    if o.expect_sha256 is not None and sha != o.expect_sha256:
        return _fail(L.FAIL_EXPECT, "computed %s, expected %s" % (sha[:12], o.expect_sha256[:12]), man["key"])
    if enforce_pins:
        pin = o.cache_listing
        if not pin:
            return _fail(L.FAIL_PIN, "strict and worker runs need the deployment's pin for this artifact", man["key"])
        if pin.get("key") and pin["key"] != man["key"]:
            return _fail(L.FAIL_KEY_MISMATCH, "the pin is for key %s, the artifact's manifest says %s" % (str(pin["key"])[:12], man["key"][:12]), man["key"])
        bad = _pin_problem(sha, size, pin)
        if bad:
            return _fail(L.FAIL_PIN, bad, man["key"])
    key_checked = False
    if o.source:
        try:
            comp = f.compute_key(o.source, o.source_id, o.allow_dirty)
        except L.UnrealError as e:
            return _fail(e.cls, e.detail)
        if comp["key"] != man["key"]:
            return _fail(L.FAIL_KEY_MISMATCH, "the artifact is for key %s, this source and recipe produce %s" % (man["key"][:12], comp["key"][:12]), man["key"])
        key_checked = True
    return Decision("use_artifact", "", "artifact %s" % man["key"][:12], man["key"], sha256=sha, size=size, key_verified=key_checked,
                    created_utc=man["created_utc"], record_sha256=True, pin_enforced=enforce_pins)
