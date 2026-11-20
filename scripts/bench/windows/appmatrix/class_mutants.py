#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Public release/transient runtime boundaries: each mutant must have two named killing tests."""
import concurrent.futures
import json
import sys
import shutil
import tempfile
import bundle_mutants as M

CASES=[
 ('public-package-boundary','package_bundle.py','if bundle_class == "public" and pid not in PUBLIC_PACKAGES:', 'if False:'),
 ('public-publisher-only','publish_bundle.py','if receipt.get("bundle_class") != "public" or any(p["id"] not in B.PUBLIC_PACKAGES or p["compliance"]["distribution"] != "public-ok" for p in receipt["packages"]):','if False:'),
 ('release-class-before-write', 'package_bundle.py', 'if bundle_class != "public": raise ValueError("release image must be public; KFAPPSP is transient-only")', 'if False: raise ValueError()'),
 ('permissive-wheel-only','bundle_wheels.py','def permissive(expression):','def permissive(expression):\n    return True\n\ndef unused_permissive(expression):'),
 ('wheel-nvidia-content','bundle_wheels.py','if NVIDIA.search(data):','if False:'),
 ('wheel-reviewed-inventory','bundle_wheels.py',"if inspect(path, pin, component) != pin.get('public_audit'):",'if False:'),
 ('missing-official-notrun','winapps.py','if required_official and (not self.official_attached or not self.official_cd):','if False:'),
 ('unknown-legacy-license','bundle_wheels.py',"if raw_license and not recognized: raise ValueError('unclear legacy licence field')", 'if False: raise ValueError()'),
 ('media-retry-class','../../box/windows_apps.sh','elif [ "${6:-0}" = 1 ]; then echo "RETRY MEDIA";', 'elif false; then echo "RETRY MEDIA";'),
 ('drive-letter-conflicts','guest/kf_guest_setup.ps1','if ($occupied) {','if ($false) {'),
 ('fetch-base-default','fetch_public_bundle.py',"if value is None: return PUBLIC_BASE","if value is None: return ''"),
 ('fetch-base-only-the-bundles-domain','fetch_public_bundle.py',"if value.rstrip('/') != PUBLIC_BASE:","if False:"),
 ('stage-never-passes-an-empty-base','../../box/windows_apps.sh','[ -z "${KF_WINAPPS_PUBLIC_BASE_URL:-}" ] || base_args=(--base "$KF_WINAPPS_PUBLIC_BASE_URL")','base_args=(--base "${KF_WINAPPS_PUBLIC_BASE_URL:-}")'),
 ('stage-default-official-driver','../../box/windows_apps.sh','DEFAULT_OFFICIAL_DRIVER=580.88  ','DEFAULT_OFFICIAL_DRIVER=580.65  '),
 ('fetch-reason-recipe-drift-not-named','fetch_public_bundle.py',"and str(error) in RECIPE_INPUT_DRIFT:","and False:"),
 ('fetch-reason-pin-mismatch-not-named','fetch_public_bundle.py',"and str(error) in PIN_MISMATCH:","and False:"),
 ('fetch-reason-network-not-named','fetch_public_bundle.py',"if isinstance(error, OSError): return 'NETWORK'","if False: return 'NETWORK'"),
 ('fetch-reason-prints-the-exception','fetch_public_bundle.py',"print('PUBLIC_BUNDLE_FAIL reason='+reason_of(error),file=sys.stderr)","print('PUBLIC_BUNDLE_FAIL reason='+reason_of(error)+' '+str(error),file=sys.stderr)"),
 ('fetch-reason-recipe-phrase-by-substring','fetch_public_bundle.py',"and str(error) in RECIPE_INPUT_DRIFT:","and any(m in str(error) for m in RECIPE_INPUT_DRIFT):"),
 ('fetch-reason-pin-phrase-by-substring','fetch_public_bundle.py',"and str(error) in PIN_MISMATCH:","and any(m in str(error) for m in PIN_MISMATCH):"),
 ('stage-pin-reasons-widened','../../box/windows_apps.sh','(RECIPE_INPUT_DRIFT|PIN_MISMATCH)$/','([A-Z_]*)$/'),
 ('stage-pin-refusal-is-a-media-retry','../../box/windows_apps.sh','        refuse_pin "$why"\n','        true\n'),
 ('stage-pin-refusal-line','../../box/windows_apps.sh','say "WINAPPS_REFUSED class=PIN reason=$1"','say "WINAPPS_REFUSED why=$1"'),
 ('stage-pin-refusal-exit-code','../../box/windows_apps.sh','reason=$1"; VERDICT=REFUSED; rc_final=2; exit 2; }','reason=$1"; VERDICT=REFUSED; rc_final=3; exit 3; }'),
 ('stage-failed-fetch-not-routed','../../box/windows_apps.sh','        fetch_failed "$EV/fetch_bundle.txt"\n','        verdict RETRY MEDIA "fetch of winapps-open.iso failed (fetch_bundle.txt)"; exit 3\n'),
 ('public-domain-required','publish_bundle.py','if not C.https(base) or "?" in base or host.endswith((".r2.dev", ".r2.cloudflarestorage.com")):','if False:'),
]

def main():
 M.COLLECT=M.COLLECT.replace('test_package_bundle.py','test_bundle_classes.py')
 original=M.fixture
 def fixture(parent,name):
  root=original(parent,name);dst=root.parents[3]/'scripts/bench/box/windows_apps.sh';dst.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(M.HERE.parents[3]/'scripts/bench/box/windows_apps.sh',dst);return root
 M.fixture=fixture
 with tempfile.TemporaryDirectory(prefix='kf-bundle-class-mutants-') as parent:
  baseline=M.run(M.fixture(parent,'baseline'))
  if baseline['killers']:raise RuntimeError('BASELINE RED: '+str(baseline['killers']))
  with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:rows=list(pool.map(lambda c:M.one(parent,c),CASES))
 print(json.dumps(dict(tests=baseline['tests'],cases=rows),indent=2))
 return int(any(c['count']<2 for c in rows))
if __name__=='__main__':sys.exit(main())
