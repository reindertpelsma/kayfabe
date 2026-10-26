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
