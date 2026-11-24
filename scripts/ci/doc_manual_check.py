#!/usr/bin/env python3
"""Structure and wording checks of the published user manual (docs/howto, docs/FAQ.md).

STATUS: LIVE, 2026-11-20 (design: docs/design/V3_PACKAGING_AND_DISTRIBUTION.md section 11).

  * every page under docs/howto/ is listed in docs/howto/SUMMARY.md exactly once, and every entry exists;
  * the pages of Parts 1 and 2 (everything outside docs/howto/ops/) and docs/FAQ.md carry no internal evidence label
    in their visible text: CODE READ, MEASURED, NOT RUN, INFERRED, OWNER DECISION, DESIGN-ONLY, "[measured", "[inferred",
    "Verified on:" or "How known:". HTML comments (the `<!-- src: ... -->` pointers) are not visible and are skipped.
    The operator pages (docs/howto/ops/) keep their labels: their readers deploy and audit the CI.

    python3 scripts/ci/doc_manual_check.py        # exit 0 / 1, one line per finding
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LABELS = re.compile(
    r"CODE READ|MEASURED|NOT RUN|INFERRED|OWNER DECISION|DESIGN-ONLY|\[measured|\[inferred|Verified on:|How known:"
)


def visible(text: str) -> str:
    return re.sub(r"<!--.*?-->", "", text, flags=re.S)


def check(root: Path = ROOT) -> list[str]:
    errors = []
    howto = root / "docs/howto"
    summary = (howto / "SUMMARY.md").read_text()
    listed = re.findall(r"\]\(([^)\s]+\.md)\)", summary)
    for name in sorted({n for n in listed if listed.count(n) > 1}):
        errors.append("SUMMARY.md lists %s more than once" % name)
    pages = {p.relative_to(howto).as_posix() for p in howto.rglob("*.md")} - {"SUMMARY.md"}
    for name in sorted(pages - set(listed)):
        errors.append("docs/howto/%s is not listed in SUMMARY.md" % name)
    for name in sorted(set(listed) - pages):
        errors.append("SUMMARY.md lists %s, which does not exist" % name)
    user = [howto / n for n in sorted(pages) if not n.startswith("ops/")] + [root / "docs/FAQ.md"]
    for path in user:
        if not path.is_file():
            continue
        in_fence = False
        for number, line in enumerate(path.read_text().splitlines(), 1):
            if line.lstrip().startswith("```"):
                in_fence = not in_fence
            hit = LABELS.search(visible(line)) if not in_fence else None
            if hit:
                errors.append("%s:%d: internal label %r in a user page" % (path.relative_to(root), number, hit[0]))
    return errors


def main() -> int:
    errors = check()
    for line in errors:
        print("MANUAL_CHECK_FAIL " + line)
    print("MANUAL_CHECK %s" % ("FAIL" if errors else "OK"))
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
