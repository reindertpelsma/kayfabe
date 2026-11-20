#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Markdown table of the UI text lines of the image B launch rows that talk about missing online add-ons (stdlib only, GPU-free).

STATUS: LIVE, 2026-11-20 (TEST BRANCH tool; first real input is the Vast run of the new full B image).

    ui_table.py DIR [DIR ...] [--max-rows N]

Reads the evidence the stage writes: every `uitext_hits.txt` under the directories, either `<prefix>/out/<boot>/<row>/<run_id>/uitext_hits.txt`
(run id = 32 hex digits) or the copy under the evidence directory, `<evidence>/<row>/uitext_hits.txt`. The row name is the application. Prints
`| app | feature | text | severity |`:

- feature: the nearest PRECEDING control in the sibling `uitext.txt` (walk order of the UI Automation tree, parents before children) whose type
  is a heading-like one (Window, TitleBar, Header, HeaderItem, TabItem, Tab, Group) or whose AutomationId says title, heading or header; `-` when
  there is none (a heuristic: it names the nearest section, not a proven owner of the text).
- severity: high (requires internet, internet connection, not available, unavailable, sign in required, trial expired/ends/days, offline), medium
  (download, install, model, add-on, plug-in, update available, content, library), low for the rest. Judged on the control's name, not its id.
- one row per application and text (case-insensitive), sorted by severity, then application, then order of appearance, at most 200 rows.
- everything is scrubbed again with the host scrub of the harness (`propimage_apps_b.scrub_text`: e-mail-shaped, long hex or GUID-like, key-shaped
  strings, text after Signed in as / Welcome / Hello); the files are text names of controls, never a screenshot."""
import argparse
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from propimage_apps_b import scrub_text  # noqa: E402

RUN_ID = re.compile(r"[0-9a-f]{32}")
MAX_ROWS = 200
MAX_FILES = 400
MAX_FILE_BYTES = 256 * 1024
MAX_TEXT = 200
HIGH = re.compile(r"requires internet|internet connection|not available|unavailable|sign in required|trial (?:expired|ends|days)|offline", re.I)
MEDIUM = re.compile(r"download|install|model|add-on|plug-in|update available|content|library", re.I)
HEADING_TYPES = ("Window", "TitleBar", "Header", "HeaderItem", "TabItem", "Tab", "Group")
HEADING_ID = re.compile(r"title|heading|header", re.I)
SEVERITIES = ("high", "medium", "low")
HEADER = ("| app | feature | text | severity |", "|---|---|---|---|")


def parse_line(line):
    """(control type, name, automation id) of one `ControlType | Name | AutomationId` line; a line that does not look like one is all name."""
    parts = line.rstrip("\r\n").split(" | ")
    if len(parts) >= 3:
        return parts[0].strip(), " | ".join(parts[1:-1]).strip(), parts[-1].strip()
    if len(parts) == 2:
        return parts[0].strip(), parts[1].rstrip(" |").strip(), ""
    return "", line.strip(), ""


def severity(text):
    return "high" if HIGH.search(text) else "medium" if MEDIUM.search(text) else "low"


def is_heading(ctype, aid):
    return ctype in HEADING_TYPES or bool(aid and HEADING_ID.search(aid))


def app_of(path):
    """The row name: the directory above the run id directory, or the directory itself in the evidence copy layout."""
    p = Path(path)
    return p.parent.parent.name if RUN_ID.fullmatch(p.parent.name) else p.parent.name


def read_lines(path):
    """The scrubbed lines of one bounded, regular, non-link text file; [] when it is none of those."""
    p = Path(path)
    try:
        if p.is_symlink() or not p.is_file() or p.stat().st_size > MAX_FILE_BYTES:
            return []
        text = p.read_bytes().decode("utf-8-sig", "replace")
    except OSError:
        return []
    return scrub_text(text).splitlines()


def find_files(roots):
    out = []
    for root in roots:
        r = Path(root)
        found = [r] if r.is_file() and r.name == "uitext_hits.txt" else sorted(r.rglob("uitext_hits.txt")) if r.is_dir() else []
        out += [p for p in found if not any(q.is_symlink() for q in (p, *p.parents[:3]))]
    return sorted(dict.fromkeys(out))[:MAX_FILES]


def features(hit_lines, context_lines):
    """For every hit line the nearest preceding heading-like control of the context (uitext.txt, else the hit file itself), else `-`."""
    context = context_lines or hit_lines
    heading_at, last = [], "-"
    for line in context:
        ctype, name, aid = parse_line(line)
        if name and is_heading(ctype, aid):
            last = name
        heading_at.append(last)
    index = {}
    for i, line in enumerate(context):
        index.setdefault(line, i)
    out = []
    for line in hit_lines:
        i = index.get(line)
        out.append(heading_at[i - 1] if i else "-")        # the heading BEFORE the line (a heading line is not its own feature)
    return out


def collect(files):
    """[(app, feature, text, severity)] in order of appearance, one per application and text."""
    seen, rows = set(), []
    for f in files:
        app = app_of(f)
        hits = [line for line in read_lines(f) if line.strip()]
        feats = features(hits, read_lines(Path(f).with_name("uitext.txt")))
        for line, feature in zip(hits, feats):
            _, name, _ = parse_line(line)
            name = name[:MAX_TEXT]
            key = (app, name.lower())
            if not name or key in seen:
                continue
            seen.add(key)
            rows.append((app, feature[:80], name, severity(name)))
    return rows


def cell(text):
    return scrub_text(text).replace("|", "\\|").replace("\n", " ").strip() or "-"


def render(rows, max_rows=MAX_ROWS):
    ordered = sorted(enumerate(rows), key=lambda e: (SEVERITIES.index(e[1][3]), e[1][0], e[0]))
    lines = list(HEADER)
    for _, (app, feature, text, sev) in ordered[:max_rows]:
        lines.append("| %s | %s | %s | %s |" % (cell(app), cell(feature), cell(text), sev))
    if len(ordered) > max_rows:
        lines.append("")
        lines.append("(%d more rows not shown)" % (len(ordered) - max_rows))
    return "\n".join(lines) + "\n"


def main(argv=None):
    p = argparse.ArgumentParser(prog="ui_table.py", description=__doc__.split("\n")[0])
    p.add_argument("dirs", nargs="+", help="evidence directories (the stage's out tree or its evidence directory)")
    p.add_argument("--max-rows", type=int, default=MAX_ROWS)
    a = p.parse_args(argv)
    sys.stdout.write(render(collect(find_files(a.dirs)), max(1, min(a.max_rows, MAX_ROWS))))
    return 0


if __name__ == "__main__":
    sys.exit(main())
