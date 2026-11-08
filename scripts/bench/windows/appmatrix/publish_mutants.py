#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Offline publication mutants; every case requires two distinct named killers."""
import concurrent.futures
import json
from pathlib import Path
import shutil
import sys
import tempfile

import bundle_mutants as M

CASES = [
    ("builder-credentials", "package_bundle.py", 'if key in env:', 'if False:'),
    ("recipe-files-hashed", "package_bundle.py", 'for name in ("package_bundle.py", "bundle_iso.py", "bundle_tools.py", "bundle_compliance.py", "bundle_wheels.py"):', 'for name in ():'),
    ("probe-sources-hashed", "package_bundle.py", 'for name in bundle_tools.SOURCES:', 'for name in ():'),
    ("guest-and-notices-hashed", "package_bundle.py", 'contents = repo_files(root)', 'contents = {}'),
    ("receipt-input-binding", "package_bundle.py", 'if receipt.get("inputs") != embedded.get("inputs"):', 'if False:'),
    ("changed-input-plan", "bundle_ci.py", 'published.get("inputs") != inputs', 'False'),
    ("equal-input-plan", "bundle_ci.py", 'published.get("inputs") != inputs', 'True'),
    ("disk-admission", "bundle_ci.py", 'if free < required:', 'if False:'),
    ("two-build-equality", "bundle_ci.py", 'if receipts[0]["image"]["sha256"] != receipts[1]["image"]["sha256"]:', 'if False:'),
    ("verify-pin-equality", "bundle_ci.py", 'if verify and not matches:', 'if False:'),
    ("publish-master-context", "publish_bundle.py", 'env.get("GITHUB_REF") != "refs/heads/master"', 'False'),
    ("publish-repository-context", "publish_bundle.py", 'env.get("GITHUB_REPOSITORY") != "reindertpelsma/kayfabe"', 'False'),
    ("publish-event-context", "publish_bundle.py", 'env.get("GITHUB_EVENT_NAME") not in ("push", "workflow_dispatch")', 'False'),
    ("object-key-digest", "publish_bundle.py", 'if not re.fullmatch(r"[0-9a-f]{64}", digest):', 'if False:'),
    ("part-size-bound", "publish_bundle.py", 'if size <= 0 or not MIN_PART <= part_size <= MAX_PART:', 'if False:'),
    ("part-count-bound", "publish_bundle.py", 'if count > 10000:', 'if False:'),
    ("embedded-xml-error", "publish_bundle.py", 'if root.tag.rsplit("}", 1)[-1] == "Error":', 'if False:'),
    ("signature-service", "publish_bundle.py", 'for value in (stamp[:8], region, "s3", "aws4_request"):', 'for value in (stamp[:8], region, "ec2", "aws4_request"):'),
    ("signature-body", "publish_bundle.py", '"x-amz-content-sha256": hashlib.sha256(data).hexdigest()', '"x-amz-content-sha256": hashlib.sha256(b"").hexdigest()'),
    ("existing-identity", "publish_bundle.py", 'if not matches(head, digest, inputs, size):', 'if False:'),
    ("resume-binding", "publish_bundle.py", 'if any(state.get(k) != v for k, v in binding.items()):', 'if False:'),
    ("resume-remote-verification", "publish_bundle.py", 'if (saved.get("sha256") == part_sha and number in remote\n                    and remote[number] == (saved.get("etag"), length)):', 'if str(number) in known:'),
    ("uploaded-whole-digest", "publish_bundle.py", 'if whole.hexdigest() != digest:', 'if False:'),
    ("complete-create-only", "publish_bundle.py", '{"If-None-Match": "*", "Content-Type": "application/xml"}', '{"Content-Type": "application/xml"}'),
    ("receipt-create-only", "publish_bundle.py", '**{"If-None-Match": "*", "Content-Type": "application/json"}', '**{"Content-Type": "application/json"}'),
    ("artifact-preflight", "publish_bundle.py", 'if Path(iso).stat().st_size != receipt["image"]["size"] or B.sha256(iso) != digest:', 'if False:'),
    ("presigned-https-key", "publish_bundle.py", 'if u.scheme != "https" or not u.hostname or u.username or u.password or u.fragment or not urllib.parse.unquote(u.path).endswith("/" + key):', 'if False:'),
    ("presigned-condition-binding", "publish_bundle.py", 'if "If-None-Match" in headers and "if-none-match" not in q.get("X-Amz-SignedHeaders", "").lower().split(";"):', 'if False:'),
    ("bounded-upload-memory", "publish_bundle.py", 'if part_size > DEFAULT_PART:', 'if False:'),
    ("aborted-upload-state", "publish_bundle.py", 'state_path.unlink(missing_ok=True)\n        head = existing(store, key)', 'pass\n        head = existing(store, key)'),
]


def main():
    # The shared harness copies sources, licenses and the real offline tests. It
    # never receives credentials or URLs and never starts an uploader transport.
    M.COLLECT = M.COLLECT.replace("test_package_bundle.py", "test_bundle_publish.py")
    original = M.fixture
    def fixture(parent, name):
        root = original(parent, name)
        dst = root.parents[3] / ".github/workflows/bundle-publish.yml"
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(M.HERE.parents[3] / ".github/workflows/bundle-publish.yml", dst)
        return root
    M.fixture = fixture
    with tempfile.TemporaryDirectory(prefix="kf-bundle-publish-mutants-") as parent:
        baseline = M.run(fixture(parent, "baseline"))
        if baseline["killers"]: raise RuntimeError("BASELINE RED: " + str(baseline["killers"]))
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            rows = list(pool.map(lambda case: M.one(parent, case), CASES))
    print(json.dumps(dict(tests=baseline["tests"], cases=rows), indent=2))
    return int(any(r["count"] < 2 for r in rows))


if __name__ == "__main__":
    sys.exit(main())
