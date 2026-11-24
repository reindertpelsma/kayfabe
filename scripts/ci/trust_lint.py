#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Trust lint: no download without an anchor (STATUS: LIVE, 2026-11-17; stdlib only; docs/design/V3_STANDALONE_TRUST.md).

Reads scripts/ci/trust_manifest.json (the repository's list of what it downloads, each artifact with its anchors) and the
scripts under scripts/ and tools/. A finding is a named line `TRUST_LINT <CODE> <where>: <what>`; exit 1 if there is any.

  FORBIDDEN_ANCHOR     an anchor of class git-sha256-of-our-own-artifact (a sha256 in git of something we build or host)
  NO_ANCHOR            an artifact with no anchor
  MANIFEST_INVALID     anything else kf_trust.validate_manifest refuses (bad class, https, pin fields, same-origin checksum)
  CONSUMER_MISSING     a file the manifest names as the consumer of an artifact does not exist
  NOT_ROUTED           a consumer of a helper-routed artifact that neither mentions kf_trust nor sits beside a consumer that does
  RAW_FETCH_OF_ROUTED  a helper-routed consumer that still spells the artifact's origin URL itself
  PIN_DRIFT            a string the manifest says a pin file holds is not in it any more (the code and the manifest drifted)
  UNROUTED_DOWNLOAD    a script with a download primitive (curl, wget, urlopen, pip install, git clone/fetch ...) that is neither a
                       routed consumer nor on the `unrouted` list, or more primitives than the list allows
  STALE_UNROUTED       an `unrouted` entry that no longer matches the file (gone, fewer primitives, or now routed): lower or
                       delete it, never raise it to make a finding pass
  BAD_UNROUTED         an `unrouted` entry without count, class, reason, or with an unknown class
  DOCKERFILE_UNPINNED  a Dockerfile FROM that is not pinned by @sha256:<digest> (or an earlier stage)

`unrouted` is a ratchet, not a licence: each entry says why the file may still download, and the count can only go down.
Tests: scripts/ci/test_trust_lint.py; mutants: scripts/ci/trust_lint_mutants.py.
"""
import argparse
import ast
import io
import json
from pathlib import Path
import re
import sys
import tokenize

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import kf_trust  # noqa: E402

ROOT = HERE.parents[1]
SCAN_DIRS = ("scripts", "tools")
SKIP_PARTS = {"__pycache__", "tests", "test", "fixtures", "node_modules", ".git", "target", "third_party", "archive"}
EXEMPT = {"scripts/ci/kf_trust.py"}  # the one place that may download: it IS the anchor check
CLASSES = ("transport", "operator", "user-input", "experiment-only", "evidence", "planned", "native-route3", "tooling")
# curl and wget count where they are invoked (an option, a quote, a variable or a URL follows), not as a package name in an apt list
_ARG = r"\s+(?:-|\"|'|\$|https?://)"
SHELL_PRIMS = re.compile(r"\bcurl" + _ARG + r"|\bwget" + _ARG + r"|[\"']curl[\"']\s*,|\baria2c\b|\bgit\s+(?:-C\s+\S+\s+)?(?:clone|fetch|submodule)\b|\bpip3?\s+(?:install|download)\b|\bpython3?\s+-m\s+pip\b")
PY_PRIMS = re.compile(r"\burlopen\b|\burlretrieve\b|\burllib\.request\b|\bhttp\.client\b|\brequests\.(?:get|post|put)\b")


def is_python(path, text):
    return path.suffix == ".py" or (path.suffix == "" and text.startswith("#!") and "python" in text.split("\n", 1)[0])


def code_lines(path, text):
    """The lines of a file that are code: no whole-line comments, and for Python no docstrings."""
    lines = text.splitlines()
    if is_python(path, text):
        blank = set()
        try:
            for node in ast.walk(ast.parse(text)):
                if isinstance(node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)) and node.body:
                    first = node.body[0]
                    if isinstance(first, ast.Expr) and isinstance(getattr(first, "value", None), ast.Constant) and isinstance(first.value.value, str):
                        blank.update(range(first.lineno - 1, first.end_lineno))
            for tok in tokenize.generate_tokens(io.StringIO(text).readline):
                if tok.type == tokenize.COMMENT:
                    blank.add(tok.start[0] - 1)
        except (SyntaxError, tokenize.TokenError, ValueError):
            pass
        return [l for i, l in enumerate(lines) if i not in blank]
    return [l for l in lines if not l.lstrip().startswith("#")]


def primitives(path, text):
    """How many code lines of this file use a download primitive."""
    prims = [SHELL_PRIMS] + ([PY_PRIMS] if is_python(path, text) else [])
    return sum(1 for l in code_lines(path, text) if any(p.search(l) for p in prims))


def scannable(rel, path):
    parts = set(rel.parts)
    if parts & SKIP_PARTS or str(rel) in EXEMPT:
        return False
    name = rel.name
    if name.startswith("test_") or name.endswith("_mutants.py") or name.endswith(".md"):
        return False
    if "Dockerfile" in name:
        return True
    if rel.suffix in (".sh", ".bash", ".py"):
        return True
    if rel.suffix == "":
        try:
            return path.read_bytes()[:2] == b"#!"
        except OSError:
            return False
    return False


def scan(root):
    """{relative path: primitive count} for every scanned file that uses a download primitive."""
    found = {}
    for d in SCAN_DIRS:
        base = root / d
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*")):
            if not path.is_file() or path.is_symlink():
                continue
            rel = path.relative_to(root)
            if not scannable(rel, path):
                continue
            try:
                text = path.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            n = primitives(path, text)
            if n:
                found[str(rel)] = n
    return found


def read(root, rel):
    try:
        return (root / rel).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None


def lint(root=ROOT, manifest_path=None):
    """All findings of the tree at root, as (code, where, text) tuples."""
    root = Path(root)
    mp = Path(manifest_path) if manifest_path else root / "scripts/ci/trust_manifest.json"
    out = []
    try:
        doc = json.loads(mp.read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        return [("MANIFEST_INVALID", mp.name, "cannot read the manifest: %s" % type(e).__name__)]
    arts = doc.get("artifacts") if isinstance(doc, dict) else None
    if not isinstance(arts, dict):
        return [("MANIFEST_INVALID", mp.name, "artifacts must be an object")]
    # 1. the anchors, before anything else is believed
    for name, e in sorted(arts.items()):
        anchors = e.get("anchors") if isinstance(e, dict) else None
        if not anchors:
            out.append(("NO_ANCHOR", name, "the artifact has no anchor"))
            continue
        for a in anchors:
            if isinstance(a, dict) and a.get("class") == kf_trust.CLASS_FORBIDDEN:
                out.append(("FORBIDDEN_ANCHOR", name, "a sha256 in git of an artifact we build or host is not an anchor"))
    seen = {(c, w) for c, w, _ in out}
    for problem in kf_trust.validate_manifest(doc):
        name = problem.split(":", 1)[0].replace("artifact ", "").split(" anchor")[0]
        if ("NO_ANCHOR", name) in seen or ("FORBIDDEN_ANCHOR", name) in seen:
            continue
        out.append(("MANIFEST_INVALID", name, problem))
    # 2. consumers, routing and pin drift
    for name, e in sorted(arts.items()):
        if not isinstance(e, dict):
            continue
        consumers = [c for c in e.get("consumers", []) if isinstance(c, str)]
        texts = {c: read(root, c) for c in consumers}
        for c, t in texts.items():
            if t is None:
                out.append(("CONSUMER_MISSING", name, c))
        routed = {c for c, t in texts.items() if t and "kf_trust" in t}
        for c, t in texts.items():
            if t is None:
                continue
            if e.get("routing") == "helper":
                beside = any(Path(r).name in t for r in routed if r != c)
                if c not in routed and not beside:
                    out.append(("NOT_ROUTED", name, "%s does not go through kf_trust" % c))
                origin = e.get("origin", "")
                if origin and origin in "\n".join(code_lines(Path(c), t)) and c not in EXEMPT:
                    out.append(("RAW_FETCH_OF_ROUTED", name, "%s spells the origin URL itself" % c))
        for pf in e.get("pin_files", []) if isinstance(e.get("pin_files"), list) else []:
            t = read(root, pf.get("file", ""))
            if t is None:
                out.append(("PIN_DRIFT", name, "%s is missing" % pf.get("file")))
                continue
            for s in pf.get("contains", []):
                if s not in t:
                    out.append(("PIN_DRIFT", name, "%s no longer holds %s" % (pf["file"], s[:24])))
        if e.get("kind") == "oci-image":
            for a in e.get("anchors", []):
                d = a.get("digest")
                for c, t in texts.items():
                    froms = [l for l in (t or "").splitlines() if re.match(r"\s*FROM\s", l)]
                    if d and not any(d in l and e.get("origin", "").split("/", 1)[-1] in l for l in froms):
                        out.append(("PIN_DRIFT", name, "%s has no FROM of %s@%s" % (c, e.get("origin"), d[:19])))
    # 3. the scan: every download primitive is routed, or on the ratchet
    unrouted = doc.get("unrouted", {})
    if not isinstance(unrouted, dict):
        out.append(("BAD_UNROUTED", "unrouted", "must be an object"))
        unrouted = {}
    for path, u in sorted(unrouted.items()):
        if not isinstance(u, dict) or type(u.get("count")) is not int or u["count"] < 1 or u.get("class") not in CLASSES \
                or not isinstance(u.get("reason"), str) or len(u.get("reason", "")) < 12:
            out.append(("BAD_UNROUTED", path, "needs count >= 1, one of %s and a reason" % (CLASSES,)))
    found = scan(root)
    # a key ending in "/" is a directory entry: its count is the total of the directory's files, which have no entry of their own
    dirs = {k: u for k, u in unrouted.items() if k.endswith("/")}
    totals = {k: 0 for k in dirs}
    for path, n in sorted(found.items()):
        owner = next((k for k in dirs if path.startswith(k)), None)
        if owner is not None:
            totals[owner] += n
            if path in unrouted:
                out.append(("BAD_UNROUTED", path, "listed beside its directory entry %s" % owner))
            continue
        u = unrouted.get(path)
        if u is None:
            out.append(("UNROUTED_DOWNLOAD", path, "%d download primitive line(s), not routed through kf_trust and not on the unrouted list" % n))
        elif isinstance(u, dict) and type(u.get("count")) is int and n > u["count"]:
            out.append(("UNROUTED_DOWNLOAD", path, "%d primitive line(s), the list allows %d: a new download needs an anchor" % (n, u["count"])))
        elif isinstance(u, dict) and type(u.get("count")) is int and n < u["count"]:
            out.append(("STALE_UNROUTED", path, "%d primitive line(s) left, the list says %d: lower the count" % (n, u["count"])))
    for k, n in sorted(totals.items()):
        u = dirs[k]
        if not isinstance(u, dict) or type(u.get("count")) is not int:
            continue
        if n > u["count"]:
            out.append(("UNROUTED_DOWNLOAD", k, "%d primitive line(s) under this directory, the list allows %d: a new download needs an anchor" % (n, u["count"])))
        elif n < u["count"]:
            out.append(("STALE_UNROUTED", k, "%d primitive line(s) left under this directory, the list says %d: lower the count" % (n, u["count"])))
    for path in sorted(unrouted):
        if path not in found and path not in dirs:
            out.append(("STALE_UNROUTED", path, "no download primitive left in this file (or it is gone): delete the entry"))
    # 4. Dockerfiles
    for d in SCAN_DIRS:
        base = root / d
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*")):
            rel = path.relative_to(root)
            if "Dockerfile" not in path.name or not path.is_file() or set(rel.parts) & SKIP_PARTS:
                continue
            stages = set()
            for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
                m = re.match(r"\s*FROM\s+(\S+)(?:\s+AS\s+(\S+))?", line, re.I)
                if not m:
                    continue
                ref = m.group(1)
                if ref != "scratch" and ref not in stages and not re.search(r"@sha256:[0-9a-f]{64}$", ref):
                    out.append(("DOCKERFILE_UNPINNED", str(rel), "FROM %s is not pinned by @sha256:<digest>" % ref))
                if m.group(2):
                    stages.add(m.group(2))
    return out


def pending(manifest_path=None):
    """Names of the artifacts whose accepted anchor awaits the owner's confirmation (informational, never a failure)."""
    doc = json.loads(Path(manifest_path or kf_trust.MANIFEST).read_text(encoding="utf-8"))
    names = []
    for name, e in sorted(doc["artifacts"].items()):
        verdicts = [kf_trust.confirmation(a) for a in e["anchors"]]
        if verdicts and all(v == "pending-owner-confirmation" for v in verdicts):
            names.append(name)
    return names


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", default=str(ROOT))
    ap.add_argument("--manifest")
    ap.add_argument("--pending", action="store_true", help="list the artifacts that rest on a pin awaiting the owner's confirmation")
    a = ap.parse_args(argv)
    if a.pending:
        for n in pending(a.manifest or Path(a.root) / "scripts/ci/trust_manifest.json"):
            print(n)
        return 0
    findings = lint(a.root, a.manifest)
    for code, where, text in findings:
        print("TRUST_LINT %s %s: %s" % (code, where, text))
    print("TRUST_LINT %s (%d finding(s))" % ("FAIL" if findings else "OK", len(findings)))
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
