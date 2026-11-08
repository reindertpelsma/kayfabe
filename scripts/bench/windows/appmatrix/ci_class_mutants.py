#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Private CI confidentiality and per-class publication/planning falsifiers."""
import concurrent.futures
import json
import sys
import tempfile
import bundle_mutants as M

CASES=[
 ('manifest-read-only','bundle_ci.py','_, body = store.request("GET", P.keys(digest, bundle_class)[1])', '_, body = store.request("POST", P.keys(digest, bundle_class)[1])'),
 ('manifest-service-fail-closed','bundle_ci.py','if error.status == 404: return None', 'return None'),
 ('two-distinct-buckets','publish_bundle.py','if "BUNDLE_OTHER_BUCKET" in env and (not env["BUNDLE_OTHER_BUCKET"] or env.get("R2_BUCKET") == env["BUNDLE_OTHER_BUCKET"]):', 'if False:'),
 ('actions-explicit-build','package_bundle.py','if env.get("GITHUB_ACTIONS") == "true" and not allow_ci:', 'if False:'),
 ('private-pin-digest-only','package_bundle.py','if bundle_class == "private": return f"{filename} sha256={image_sha}\\n"', 'if False: return ""'),
 ('private-object-namespace','publish_bundle.py','prefix = "winapps-open" if bundle_class == "public" else "winapps-private"', 'prefix = "winapps-open"'),
 ('private-cache-log','package_bundle.py','if private else f"CACHE_OK', 'if False else f"CACHE_OK'),
 ('private-error-redaction','package_bundle.py','print("FAIL package=" + package_id if args.bundle_class == "private" else "FAIL " + str(e), file=sys.stderr)', 'print("FAIL " + str(e), file=sys.stderr)'),
 ('manifest-class-binding','bundle_ci.py','receipt.get("bundle_class") != bundle_class', 'False'),
 ('manifest-pin-binding','bundle_ci.py','receipt.get("image", {}).get("sha256") != published.get("sha256")', 'False'),
 ('private-destination-class','publish_bundle.py','elif bundle_class != "private" or receipt.get("bundle_class") != "private" or any(p["id"] not in B.OPEN_PACKAGES - B.PUBLIC_PACKAGES or p["compliance"]["distribution"] != "private-only" for p in receipt["packages"]):', 'elif False:'),
]

def main():
 M.COLLECT=M.COLLECT.replace('test_package_bundle.py','test_bundle_ci_classes.py')
 with tempfile.TemporaryDirectory(prefix='kf-bundle-ci-class-mutants-')as parent:
  baseline=M.run(M.fixture(parent,'baseline'))
  if baseline['killers']:raise RuntimeError('BASELINE RED: '+str(baseline['killers']))
  with concurrent.futures.ThreadPoolExecutor(max_workers=2)as pool:rows=list(pool.map(lambda c:M.one(parent,c),CASES))
 print(json.dumps(dict(tests=baseline['tests'],cases=rows),indent=2))
 return int(any(c['count']<2 for c in rows))

if __name__=='__main__':sys.exit(main())
