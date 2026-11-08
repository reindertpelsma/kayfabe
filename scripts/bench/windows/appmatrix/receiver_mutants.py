#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""RV2 receiver mutations: two named offline falsifiers per receiving rule.

Fixtures rewrite every producer-owned hash/marker and use the real verifier,
comparison CLI and publisher. Only trusted toy inputs and transport are replaced.
"""
import concurrent.futures
import json
import sys
import tempfile

import bundle_mutants as M

CASES = [
    ("embedded-recipe-identity", "package_bundle.py", 'if derived != embedded["inputs"]:', 'if False:'),
    ("reviewed-recipe-authorization", "package_bundle.py", 'if expected_inputs is not None and derived != expected_inputs:', 'if False:'),
    ("publisher-reviewed-digest", "publish_bundle.py", 'receipt, expected_inputs=expected)', 'receipt)'),
    ("verifier-cli-reviewed-digest", "package_bundle.py", 'expected_inputs=reviewed)', 'expected_inputs=None)'),
    ("reviewed-recipe-source-audit", "package_bundle.py", 'if receipt.get("recipe_sha256") != recipe:', 'if False:'),
    ("actual-pinned-iso-bytes", "bundle_compliance.py", 'if records.get(iso_name) != {k: pin[k] for k in ("sha256", "size")}:', 'if False:'),
    ("derived-repository-and-legal-files", "package_bundle.py", 'if got.get(name) != pin:', 'if False:'),
    ("canonical-embedded-recipe", "package_bundle.py", 'canonical(dict(schema=1, inputs=derived,\n            packages=embedded["packages"], toolchain=embedded["toolchain"],\n            open_packages=sorted(embedded["open_packages"]), bundle_class=bundle_class))', 'canonical(embedded)'),
    ("reviewed-file-licenses", "package_bundle.py", 'if record["licenses"] != licenses.get(name, own):', 'if False:'),
    ("reviewed-owned-output-presence", "package_bundle.py", 'if expected_inputs is not None and not tool_paths <= set(got):', 'if False:'),
    ("reviewed-file-closure", "package_bundle.py", 'if set(got) - (set(derived_files) | package_paths | tool_paths | {"KFAPPS.ID"}):', 'if False:'),
    ("primary-joliet-file-correspondence", "bundle_iso.py", 'if set(primary_dirs) != set(alias_dirs.values()) or set(primary_files) != set(alias_files):', 'if False:'),
    ("primary-joliet-payload", "bundle_iso.py", 'if pin(actual) != pin(expected):', 'if False:'),
    ("namespace-path-table-validation", "bundle_iso.py", '        self._path_tables(descriptor, joliet)', '        pass'),
    ("path-table-directory-correspondence", "bundle_iso.py", 'if rows != {p: e[0] for p, e in self._directories.items()}:', 'if False:'),
    ("namespace-dot-references", "bundle_iso.py", 'if not isdir or (start, size) != target:', 'if False:'),
    ("member-read-bound", "bundle_iso.py", 'length = min(size, self.size - self.position)', 'length = size'),
    ("member-seek-bound", "bundle_iso.py", 'if whence not in (0, 1, 2) or not 0 <= target <= self.size:', 'if False:'),
    ("member-read-only", "bundle_iso.py", 'if mode != "rb": raise ValueError("ISO members are read-only")', 'if False: raise ValueError()'),
    ("member-closed-read", "bundle_iso.py", 'if self.closed: raise ValueError("read from closed ISO member")', 'if False: raise ValueError()'),
]


def main():
    M.COLLECT = M.COLLECT.replace("test_package_bundle.py", "test_bundle_receivers.py")
    with tempfile.TemporaryDirectory(prefix="kf-receiver-mutants-") as parent:
        baseline = M.run(M.fixture(parent, "baseline"))
        if baseline["killers"]:
            raise RuntimeError("BASELINE RED: " + str(baseline["killers"]))
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            rows = list(pool.map(lambda case: M.one(parent, case), CASES))
    print(json.dumps(dict(tests=baseline["tests"], cases=rows), indent=2))
    return int(any(row["count"] < 2 for row in rows))


if __name__ == "__main__":
    sys.exit(main())
