"""Print the generated site inventory and broken links for the Actions step summary."""

import json
from pathlib import Path

report = json.loads(Path(__file__).with_name("build-report.json").read_text())
print("Root landing: `" + report["root"] + "`; latest docs: `" + report["latest"] + "`.\n")
for item in report["counts"]:
    print(f"- `{item['version']}`: {item['manual_pages']} user manual pages")
print(f"\nBroken internal links: {len(report['broken_links'])} unique page/target pairs.\n")
for item in report["broken_links"]:
    print(f"- `{item['page']}` → `{item['target']}` ({item['reason']})")

for page, review in report.get("copy_review", {}).items():
    print("\nCopy review: `" + page + "` (sentence limits hard; Flesch/depth advisory)")
    for section in review["sections"]:
        print(f"- {section['heading']}: Flesch={section['flesch_reading_ease']}, "
              f"average sentence={section['average_sentence_words']}, "
              f"longest={section['longest_sentence_words']}, blocks={section['blocks']}")
    for warning in review["warnings"]:
        print("- Advisory: " + warning)
