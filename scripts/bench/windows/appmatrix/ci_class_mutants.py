#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Public-only release and manifest-binding falsifiers."""
import concurrent.futures
import json
import sys
import tempfile
import bundle_mutants as M

CASES=[('actions-explicit-build', 'package_bundle.py', 'if env.get("GITHUB_ACTIONS") == "true" and not allow_ci:', 'if False:'), ('release-class-guard', 'package_bundle.py', 'if bundle_class != "public": raise ValueError("permanent bundles are public-only; official media is transient")', 'if False: raise ValueError()'), ('runtime-has-no-repo-pin', 'package_bundle.py', 'if bundle_class != "public": raise ValueError("transient media has no repository pin")', 'if False: raise ValueError()'), ('runtime-has-no-object-key', 'publish_bundle.py', 'if bundle_class != "public": raise ValueError("only public release object keys are permitted")', 'if False: raise ValueError()'), ('manifest-class-binding', 'bundle_ci.py', 'receipt.get("bundle_class") != bundle_class', 'False'), ('manifest-pin-binding', 'bundle_ci.py', 'receipt.get("image", {}).get("sha256") != published.get("sha256")', 'False'), ('public-only-publication', 'publish_bundle.py', 'if bundle_class != "public": raise PublishError("only public release publication is permitted")', 'if False: raise ValueError()')]


def main():
 M.COLLECT=M.COLLECT.replace('test_package_bundle.py','test_bundle_ci_classes.py')
 with tempfile.TemporaryDirectory(prefix='kf-bundle-ci-class-mutants-')as parent:
  baseline=M.run(M.fixture(parent,'baseline'))
  if baseline['killers']:raise RuntimeError('BASELINE RED: '+str(baseline['killers']))
  with concurrent.futures.ThreadPoolExecutor(max_workers=2)as pool:rows=list(pool.map(lambda c:M.one(parent,c),CASES))
 print(json.dumps(dict(tests=baseline['tests'],cases=rows),indent=2))
 return int(any(c['count']<2 for c in rows))

if __name__=='__main__':sys.exit(main())
