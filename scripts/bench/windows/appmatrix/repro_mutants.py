#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Two-level reproduction falsifiers, each requiring two independent killers."""
import concurrent.futures,json,sys,tempfile
import bundle_mutants as M
CASES=[
 ('comparison-report-json-name', 'package_bundle.py', '            (args.out / "reproduction.json").parent.mkdir(parents=True, exist_ok=True)\n            (args.out / "reproduction.json").write_bytes(canonical(result))', '            (args.out / "reproduction.diff.json").parent.mkdir(parents=True, exist_ok=True)\n            (args.out / "reproduction.diff.json").write_bytes(canonical(result))'),
 ('added-payload','bundle_repro.py','different=bool(added or removed or changed)','different=bool(removed or changed)'),
 ('removed-payload','bundle_repro.py','different=bool(added or removed or changed)','different=bool(added or changed)'),
 ('changed-payload','bundle_repro.py','different=bool(added or removed or changed)','different=bool(added or removed)'),
 ('container-not-level1-gate','bundle_repro.py',"level1='different'if different else'equal'","level1='different'if different or not container_equal else'equal'"),
 ('vendor-no-byte-waiver','bundle_repro.py',"if actual!=expected:raise ValueError('per-file comparison differs from pinned program policy: '+path)",'if False:raise ValueError()'),
 ('qualified-program-exception','bundle_repro.py',"if not isinstance(result.get('reason'),str) or not result['reason'].strip() or not isinstance(result.get('functional_checks'),list) or not result['functional_checks'] or not all(isinstance(x,str) and re.fullmatch(r'[a-z0-9_]+',x)for x in result['functional_checks']):",'if False:'),
 ('own-object-integrity','bundle_repro.py',"if path.stat().st_size!=receipt['image']['size'] or h.hexdigest()!=receipt['image']['sha256']:",'if False:'),
 ('own-file-list','bundle_repro.py',"if set(records)!=set(receipt['files']):raise ValueError('LEVEL1_FAIL candidate file list differs from its own manifest')",'if False:raise ValueError()'),
 ('own-file-hashes','bundle_repro.py',"if any(pin[k]!=receipt['files'][name][k]for k in ('sha256','size')):",'if False:'),
 ('inventory-only-marker','package_bundle.py',"and record.get(\"reproducibility\", {}).get(\"mode\") == \"inventory-only\"):",'and False):'),
 ('fixed-artifact-url','package_bundle.py','if "latest" in url.path.lower().split("/"):','if False:'),
 ('vendor-disappearance','package_bundle.py','if e.code in (404, 410):','if False:'),
 ('equal-container-no-warning','bundle_repro.py',"level2='equal'if container_equal else'different'","level2='different'"),
]
def main():
 M.COLLECT=M.COLLECT.replace('test_package_bundle.py','test_bundle_repro.py')
 with tempfile.TemporaryDirectory(prefix='kf-repro-mutants-')as parent:
  baseline=M.run(M.fixture(parent,'baseline'))
  if baseline['killers']:raise RuntimeError('BASELINE RED '+str(baseline['killers']))
  with concurrent.futures.ThreadPoolExecutor(max_workers=2)as pool:rows=list(pool.map(lambda c:M.one(parent,c),CASES))
 print(json.dumps(dict(tests=baseline['tests'],cases=rows),indent=2));return int(any(c['count']<2 for c in rows))
if __name__=='__main__':sys.exit(main())
