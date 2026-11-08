#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Offline compliance mutants: each rule requires two distinct named killers."""
import concurrent.futures
import json
import sys
import tempfile

import bundle_mutants as M

CASES = [
    ("compliance-identity", "bundle_compliance.py", 'if not c.get("license_id") or not https(c.get("upstream", "")) or not p.get("version"):', 'if False:'),
    ("public-only", "bundle_compliance.py", 'if public_only and c["distribution"] != "public-ok":', 'if False:'),
    ("all-files-covered", "bundle_compliance.py", 'if set(rules) != {f["name"] for f in p["files"]}:', 'if False:'),
    ("copyleft-mirror", "bundle_compliance.py", 'if re.search(r"(?:^|\\W)(?:GPL|LGPL|AGPL)-", c["license_id"]) and not sources:', 'if False:'),
    ("exact-source-version", "bundle_compliance.py", 'if not isinstance(sources, list) or any(s.get("version") != p["version"] for s in sources):', 'if False:'),
    ("compliance-pins", "bundle_compliance.py", 'validate_pin(pid, pin)', 'pass'),
    ("legal-text-size", "bundle_compliance.py", 'MAX_TEXT = 4 * 1024**2', 'MAX_TEXT = 8 * 1024**2'),
    ("missing-archive-license", "bundle_compliance.py", 'if not count: raise ValueError', 'if False: raise ValueError'),
    ("verified-source-cache", "bundle_compliance.py", 'B.verify_file(owner, pin, path)', 'None'),
    ("source-offer-term", "bundle_compliance.py", 'at least three years after our last distribution', 'one year after our last distribution'),
    ("per-file-license", "package_bundle.py", 'license_records.get(name, own_licenses)', 'own_licenses'),
    ("source-before-object-code", "publish_bundle.py", '    publish_sources(store, iso, receipt, state_path, part_size, bundle_class)\n', '    pass\n'),
    ("published-source-coverage", "publish_bundle.py", 'if receipt.get("sources") != expected:', 'if False:'),
    ("source-key-prefix", "bundle_compliance.py", 'return ("winapps-open/" if bundle_class == "public" else "winapps-private/")', 'return ("other/" if bundle_class == "public" else "winapps-private/")'),
    ("main-license-with-dependencies", "bundle_compliance.py", 'if kind == "upstream" or rule.get("fallback"):', 'if kind == "upstream" or (not count and rule.get("fallback")):'),
    ("runtime-eula-names", "bundle_compliance.py", 're.search(r"licen[cs]e|copying|copyright|notice|eula|third.party|acknowledg", base)', 're.match(r"licen[cs]e|copying|copyright|notice", base)'),
    ("bounded-legal-text-read", "bundle_compliance.py", 'return source.read(MAX_TEXT + 1)', 'return source.read()'),
    ("named-archive-failure", "bundle_compliance.py", 'raise ValueError(f"package {path.parent.name}/{path.name}: invalid licence archive ({type(error).__name__})") from None', 'raise'),
    ("source-and-licence-disk-budget", "bundle_ci.py", 'compliance = sum(pin["size"] for _, pin, _ in C.inputs(packages))', 'compliance = 0'),
    ("copied-licence-byte-pin", "bundle_compliance.py", 'if pinned_text and (digest != parent_pin["sha256"] or len(body) != parent_pin["size"]):', 'if False:'),
]


def main():
    M.COLLECT = M.COLLECT.replace("test_package_bundle.py", "test_bundle_compliance.py")
    with tempfile.TemporaryDirectory(prefix="kf-bundle-compliance-mutants-") as parent:
        baseline = M.run(M.fixture(parent, "baseline"))
        if baseline["killers"]: raise RuntimeError("BASELINE RED: " + str(baseline["killers"]))
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            rows = list(pool.map(lambda case: M.one(parent, case), CASES))
    print(json.dumps(dict(tests=baseline["tests"], cases=rows), indent=2))
    return int(any(r["count"] < 2 for r in rows))


if __name__ == "__main__":
    sys.exit(main())
