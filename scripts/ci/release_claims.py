#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Validate the release claims ledger and print the release checklist (stdlib only).

STATUS: PROPOSAL, 2026-11-07. Not wired into required CI (docs/RELEASE_CLAIMS.md, "Proposed jobs").

The ledger is docs/RELEASE_CLAIMS.tsv: one row per verifiable claim that README.md makes. The
README is the contract. A README line is a claim line unless it is blank, a heading, a table
header or a table separator. Every claim line must be covered by at least one ledger row, by the
hash of its normalised text (`src_hash`), so a README edit without a ledger update FAILS. A row
whose hash no longer matches any README line is stale and FAILS too.

  release_claims.py validate   [--strict]   structure, README coverage, evidence pointers
  release_claims.py summary                 counts by kind, status, where, action
  release_claims.py checklist --rev REV     what still has to be re-verified at REV, by stage
  release_claims.py release --rev REV       validate, then exit 1 unless every claim is verified at REV
  release_claims.py lines                   README claim lines with hash, line number and covering ids
  release_claims.py renumber                rewrite the line numbers of README rows from the hashes

Exit: 0 ok, 1 findings, 2 usage or unreadable input.
"""

from __future__ import annotations

import argparse
import csv
import datetime
import hashlib
import io
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LEDGER = "docs/RELEASE_CLAIMS.tsv"
DOC = "docs/RELEASE_CLAIMS.md"
README = "README.md"

COLUMNS = [
    "claim_id", "source", "src_hash", "claim", "kind", "verification", "check_ref",
    "where", "status", "evidence", "rev", "date", "action", "note",
]
KINDS = ("auto", "manual", "wording")
WHERES = (
    "CI GPU-free",
    "master gate middle",
    "release large",
    "owner hardware",
    "not verifiable: reword",
)
STATUSES = ("verified", "outdated", "never-verified", "contradicted", "unknown")
ACTIONS = ("keep", "update-wording", "add-test", "drop")
SLUG = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
REV = re.compile(r"^[0-9a-f]{7,40}$")
DATE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
MANUAL_ID = re.compile(r"^MC-\d+$")
LINK = re.compile(r"(?<!!)\[[^\]]*\]\(([^)\s]+)\)")
REV_TOKEN = re.compile(r"`([0-9a-f]{8,9})`")


def norm(text: str) -> str:
    return " ".join(text.split())


def line_hash(line: str) -> str:
    return hashlib.sha1(norm(line).encode()).hexdigest()[:10]


def readme_claim_lines(text: str) -> list[tuple[int, str, str]]:
    """(line number, text, hash) of every README line that is a claim line."""
    lines = text.splitlines()
    out = []
    for i, raw in enumerate(lines):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if re.fullmatch(r"\|?[\s:|-]+\|?", line) and "-" in line:
            continue  # table separator row
        nxt = lines[i + 1].strip() if i + 1 < len(lines) else ""
        if line.startswith("|") and re.fullmatch(r"\|?[\s:|-]+\|?", nxt) and "-" in nxt:
            continue  # table header row
        out.append((i + 1, raw, line_hash(raw)))
    return out


def parse_ledger(text: str) -> tuple[list[dict], list[str]]:
    errors: list[str] = []
    rows: list[dict] = []
    reader = csv.reader(io.StringIO(text), delimiter="\t", quoting=csv.QUOTE_NONE)
    header = None
    for n, rec in enumerate(reader, 1):
        if not rec or rec == [""]:
            continue
        if header is None:
            header = rec
            if header != COLUMNS:
                errors.append(f"ledger:{n}: header must be exactly {COLUMNS}, got {header}")
                return rows, errors
            continue
        if len(rec) != len(COLUMNS):
            errors.append(f"ledger:{n}: {len(rec)} columns, expected {len(COLUMNS)}")
            continue
        row = dict(zip(COLUMNS, (c.strip() for c in rec)))
        row["_line"] = n
        rows.append(row)
    if header is None:
        errors.append("ledger: empty file, no header")
    return rows, errors


def split_items(value: str) -> list[str]:
    return [v.strip() for v in re.split(r"[;,]", value) if v.strip() and v.strip() != "-"]


def git_object_exists(root: Path, spec: str) -> bool:
    try:
        r = subprocess.run(
            ["git", "-C", str(root), "cat-file", "-e", spec],
            capture_output=True, timeout=30, check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return False
    return r.returncode == 0


def check_pointer(item: str, root: Path, check_git: bool) -> str | None:
    """None if the evidence pointer is fine, else the reason."""
    if item.startswith("git:"):
        parts = item.split(":", 2)
        if len(parts) != 3 or not REV.match(parts[1]) or not parts[2]:
            return f"malformed git pointer {item!r} (git:<rev>:<path>)"
        if check_git and not git_object_exists(root, f"{parts[1]}:{parts[2]}"):
            return f"git object {parts[1]}:{parts[2]} not found"
        return None
    path = item.split("#", 1)[0]
    if not path or path.startswith("/") or ".." in Path(path).parts:
        return f"evidence path {item!r} must be a relative path inside the tree"
    if not (root / path).exists():
        return f"evidence path {path!r} does not exist"
    return None


def manual_ids(doc_text: str) -> set[str]:
    return set(re.findall(r"^#{2,4}\s+(MC-\d+)\b", doc_text, flags=re.M))


def validate(
    rows: list[dict],
    readme_text: str,
    root: Path,
    doc_text: str = "",
    check_git: bool = False,
    today: datetime.date | None = None,
) -> tuple[list[str], list[str]]:
    errors: list[str] = []
    warnings: list[str] = []
    today = today or datetime.date.today()
    seen: dict[str, int] = {}
    claim_lines = readme_claim_lines(readme_text)
    by_hash = {h: (n, t) for n, t, h in claim_lines}
    covered: set[str] = set()
    mids = manual_ids(doc_text)
    for idx, row in enumerate(rows, 2):
        ln = row.get("_line", idx)
        cid = row["claim_id"]
        at = f"row {ln} ({cid or '?'})"
        if not SLUG.match(cid):
            errors.append(f"{at}: claim_id must be a lowercase slug")
        if cid in seen:
            errors.append(f"{at}: duplicate claim_id, first on row {seen[cid]}")
        seen[cid] = ln
        for col in ("claim", "verification", "source"):
            if not row[col] or row[col] == "-":
                errors.append(f"{at}: {col} is empty")
        kind, where, status, action = row["kind"], row["where"], row["status"], row["action"]
        if kind not in KINDS:
            errors.append(f"{at}: kind {kind!r} not in {KINDS}")
        if where not in WHERES:
            errors.append(f"{at}: where {where!r} not in {WHERES}")
        if status not in STATUSES:
            errors.append(f"{at}: status {status!r} not in {STATUSES}")
        if action not in ACTIONS:
            errors.append(f"{at}: action {action!r} not in {ACTIONS}")
        # kind, where and check_ref must agree
        refs = split_items(row["check_ref"])
        if kind == "wording":
            if where != "not verifiable: reword":
                errors.append(f"{at}: a wording row must have where='not verifiable: reword'")
            if status == "verified":
                errors.append(f"{at}: a wording row cannot be verified; reword it or make it auto/manual")
        elif where == "not verifiable: reword":
            errors.append(f"{at}: only a wording row may say 'not verifiable: reword'")
        if kind == "auto":
            if where == "owner hardware":
                errors.append(f"{at}: an auto claim cannot run on owner hardware only")
            if not refs:
                errors.append(f"{at}: an auto claim needs check_ref (script path or NEW:<stage>)")
            for ref in refs:
                if ref.startswith("NEW:"):
                    if not SLUG.match(ref[4:]):
                        errors.append(f"{at}: check_ref {ref!r}: NEW:<slug> expected")
                    if action == "keep" and status != "verified":
                        errors.append(f"{at}: a NEW check must be action add-test, not keep")
                elif not (root / ref).exists():
                    errors.append(f"{at}: check_ref script {ref!r} does not exist")
        elif kind == "manual":
            if where not in ("owner hardware", "release large"):
                errors.append(f"{at}: a manual claim runs on owner hardware or release large")
            if not refs:
                errors.append(f"{at}: a manual claim needs a checklist id (MC-n) in check_ref")
            for ref in refs:
                if not MANUAL_ID.match(ref):
                    errors.append(f"{at}: check_ref {ref!r}: manual claims name MC-n ids")
                elif doc_text and ref not in mids:
                    errors.append(f"{at}: checklist {ref} is not a heading of {DOC}")
        else:
            if refs:
                errors.append(f"{at}: a wording row has no check_ref")
        # status evidence rules
        revs = split_items(row["rev"])
        for r in revs:
            if not REV.match(r):
                errors.append(f"{at}: rev {r!r} is not a hex revision of 7-40 digits")
        date = row["date"]
        if date != "-" and not DATE.match(date):
            errors.append(f"{at}: date {date!r} is not YYYY-MM-DD")
        elif date != "-":
            try:
                if datetime.date.fromisoformat(date) > today:
                    errors.append(f"{at}: date {date} is in the future")
            except ValueError:
                errors.append(f"{at}: date {date!r} is not a calendar date")
        ev = split_items(row["evidence"])
        for item in ev:
            why = check_pointer(item, root, check_git)
            if why:
                errors.append(f"{at}: {why}")
        if status in ("verified", "outdated"):
            if not revs:
                errors.append(f"{at}: status {status} needs the revision it was run at")
            if date == "-":
                errors.append(f"{at}: status {status} needs the date it was run on")
            if not ev:
                errors.append(f"{at}: status {status} needs an evidence pointer")
        if status == "contradicted":
            if not ev:
                errors.append(f"{at}: status contradicted needs a pointer to the contradicting evidence")
            if not row["note"] or row["note"] == "-":
                errors.append(f"{at}: status contradicted needs a note saying what disagrees")
        if status == "never-verified" and ev:
            warnings.append(f"{at}: never-verified with evidence pointers; use outdated or verified")
        if status == "verified" and action in ("update-wording", "drop"):
            warnings.append(f"{at}: verified but action {action}; is the wording the problem?")
        # README mapping
        src = row["source"]
        m = re.fullmatch(r"([^:]+):(\d+)", src)
        if not m:
            errors.append(f"{at}: source {src!r} must be file:line")
            continue
        sfile, sline = m.group(1), int(m.group(2))
        if sfile == README:
            h = row["src_hash"]
            if h not in by_hash:
                errors.append(
                    f"{at}: src_hash {h!r} matches no README claim line (README edited or "
                    "the row is stale): update the row"
                )
            else:
                covered.add(h)
                if by_hash[h][0] != sline:
                    errors.append(
                        f"{at}: source says README.md:{sline} but that text is on line "
                        f"{by_hash[h][0]} (run: release_claims.py renumber)"
                    )
        else:
            if row["src_hash"] != "-":
                errors.append(f"{at}: only README rows carry a src_hash; use '-'")
            p = root / sfile
            if not p.is_file():
                errors.append(f"{at}: source file {sfile!r} does not exist")
            else:
                n_lines = len(p.read_text(errors="replace").splitlines())
                if sline < 1 or sline > n_lines:
                    errors.append(f"{at}: {sfile} has {n_lines} lines, not {sline}")
    for n, text, h in claim_lines:
        if h not in covered:
            errors.append(
                f"README.md:{n}: claim line not covered by any ledger row (hash {h}): "
                f"{norm(text)[:90]!r}"
            )
    return errors, warnings


def readme_link_errors(readme_text: str, root: Path) -> list[str]:
    errors = []
    for n, line in enumerate(readme_text.splitlines(), 1):
        for target in LINK.findall(line):
            if re.match(r"[a-z][a-z0-9+.-]*:", target) or target.startswith("#"):
                continue
            path, _, frag = target.partition("#")
            p = root / path
            if not p.exists():
                errors.append(f"README.md:{n}: link target {path!r} does not exist")
            elif frag and p.suffix == ".md":
                slugs = {gh_slug(h) for h in re.findall(r"^#{1,6}\s+(.*?)\s*#*\s*$", p.read_text(errors="replace"), re.M)}
                if frag not in slugs:
                    errors.append(f"README.md:{n}: anchor #{frag} not found in {path}")
    return errors


def gh_slug(heading: str) -> str:
    s = re.sub(r"[`*_]", "", heading.strip().lower())
    s = re.sub(r"[^\w\- ]", "", s)
    return s.replace(" ", "-")


def readme_rev_warnings(readme_text: str, root: Path) -> list[str]:
    out = []
    for n, line in enumerate(readme_text.splitlines(), 1):
        for rev in REV_TOKEN.findall(line):
            if not git_object_exists(root, f"{rev}^{{commit}}"):
                out.append(f"README.md:{n}: revision {rev} is not a commit in this clone")
    return out


def rev_matches(row_revs: list[str], rev: str) -> bool:
    for r in row_revs:
        a, b = (r, rev) if len(r) <= len(rev) else (rev, r)
        if len(a) >= 7 and b.startswith(a):
            return True
    return False


def classify(row: dict, rev: str) -> str:
    st = row["status"]
    if st == "verified":
        return "ok" if rev_matches(split_items(row["rev"]), rev) else "stale"
    if st in ("outdated", "contradicted"):
        return "fix"
    return "todo"


def checklist(rows: list[dict], rev: str) -> str:
    groups: dict[str, list[tuple[str, dict]]] = {w: [] for w in WHERES}
    counts = {"ok": 0, "stale": 0, "fix": 0, "todo": 0}
    for row in rows:
        c = classify(row, rev)
        counts[c] += 1
        if c != "ok":
            groups.setdefault(row["where"], []).append((c, row))
    out = [f"RELEASE CHECKLIST for revision {rev}",
           f"claims: {len(rows)}  ok at this revision: {counts['ok']}  stale (verified at another "
           f"revision): {counts['stale']}  outdated or contradicted: {counts['fix']}  "
           f"never verified or unknown: {counts['todo']}", ""]
    for where in WHERES:
        items = groups.get(where, [])
        if not items:
            continue
        out.append(f"== {where} ({len(items)})")
        for c, row in items:
            tag = {"stale": "STALE", "fix": "FIX", "todo": "TODO"}[c]
            ref = row["check_ref"] if row["check_ref"] != "-" else row["action"]
            out.append(
                f"  [{tag}] {row['claim_id']}  ({row['source']}, {row['kind']}, "
                f"status {row['status']}, action {row['action']})"
            )
            out.append(f"         run: {ref}")
        out.append("")
    return "\n".join(out)


def summary(rows: list[dict]) -> str:
    def tally(col: str, order) -> str:
        return "  ".join(f"{k}={sum(1 for r in rows if r[col] == k)}" for k in order)
    return "\n".join([
        f"claims={len(rows)}",
        "kind:    " + tally("kind", KINDS),
        "status:  " + tally("status", STATUSES),
        "where:   " + tally("where", WHERES),
        "action:  " + tally("action", ACTIONS),
    ])


def renumber(ledger_text: str, readme_text: str) -> str:
    by_hash = {h: n for n, _t, h in readme_claim_lines(readme_text)}
    out = []
    for i, line in enumerate(ledger_text.splitlines()):
        cols = line.split("\t")
        if i > 0 and len(cols) == len(COLUMNS) and cols[1].startswith(f"{README}:") and cols[2] in by_hash:
            cols[1] = f"{README}:{by_hash[cols[2]]}"
        out.append("\t".join(cols))
    return "\n".join(out) + "\n"


def load(root: Path, ledger: str, readme: str, doc: str):
    try:
        ledger_text = (root / ledger).read_text()
        readme_text = (root / readme).read_text()
    except OSError as e:
        print(f"release_claims: cannot read input: {e}", file=sys.stderr)
        sys.exit(2)
    doc_path = root / doc
    doc_text = doc_path.read_text() if doc_path.is_file() else ""
    return ledger_text, readme_text, doc_text


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("cmd", choices=["validate", "summary", "checklist", "release", "lines", "renumber"])
    ap.add_argument("--root", default=str(ROOT))
    ap.add_argument("--ledger", default=LEDGER)
    ap.add_argument("--readme", default=README)
    ap.add_argument("--doc", default=DOC)
    ap.add_argument("--rev", help="revision the release checklist is for")
    ap.add_argument("--check-git", action="store_true",
                    help="resolve git: evidence pointers and README revisions in this clone")
    args = ap.parse_args(argv)
    root = Path(args.root)
    ledger_text, readme_text, doc_text = load(root, args.ledger, args.readme, args.doc)
    rows, perrors = parse_ledger(ledger_text)
    if args.cmd == "renumber":
        (root / args.ledger).write_text(renumber(ledger_text, readme_text))
        return 0
    if args.cmd == "lines":
        ids: dict[str, list[str]] = {}
        for r in rows:
            ids.setdefault(r["src_hash"], []).append(r["claim_id"])
        for n, text, h in readme_claim_lines(readme_text):
            print(f"{n}\t{h}\t{','.join(ids.get(h, [])) or 'UNCOVERED'}\t{norm(text)[:100]}")
        return 0
    if args.cmd == "summary":
        print(summary(rows))
        return 1 if perrors else 0
    if args.cmd in ("checklist", "release") and not args.rev:
        print("release_claims: --rev is required", file=sys.stderr)
        return 2
    errors, warnings = validate(rows, readme_text, root, doc_text, args.check_git)
    errors = perrors + errors + readme_link_errors(readme_text, root)
    if args.check_git:
        warnings += readme_rev_warnings(readme_text, root)
    if args.cmd == "checklist":
        print(checklist(rows, args.rev))
        for e in errors:
            print(f"ERROR {e}")
        return 1 if errors else 0
    for w in warnings:
        print(f"WARN {w}")
    for e in errors:
        print(f"ERROR {e}")
    if args.cmd == "release":
        print(checklist(rows, args.rev))
        pending = [r for r in rows if classify(r, args.rev) != "ok"]
        if pending or errors:
            print(f"release_claims: NOT READY: {len(pending)} claims not verified at {args.rev}, "
                  f"{len(errors)} ledger errors")
            return 1
        print("release_claims: READY")
        return 0
    print(f"release_claims: {len(rows)} claims, {len(errors)} errors, {len(warnings)} warnings")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
