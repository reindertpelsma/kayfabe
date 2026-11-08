#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Official-only runtime media falsifiers; every case needs two named killers."""
import concurrent.futures,json,sys,tempfile
import bundle_mutants as M
CASES=[
 ('generated-official-pin-lock', 'gen_official.py', "sha256=f['sha256']", 'sha256="0"*64'),
 ('official-fetcher-pin','official_runtime.py',"if not path.is_file() or path.is_symlink() or B.sha256(path)!=pin['sha256']:",'if False:'),
 ('official-upstream-failure','official_runtime.py','if result.returncode:', 'if False:'),
 ('official-origin','official_runtime.py',"if not allowed:raise MediaDrift('MEDIA_DRIFT non-official origin: '+e['package'])",'if False:raise MediaDrift()'),
 ('guest-destination','official_runtime.py',"if '..'in destination.parts or not any(destination.is_relative_to(PureWindowsPath(p))for p in ('C:/kfapps','C:/kf/official')):",'if False:'),
 ('explicit-driver-selection','official_runtime.py',"if groups and not driver_version:raise MediaDrift('MEDIA_DRIFT explicit J2 Windows driver selection required')",'if False:raise MediaDrift()'),
 ('driver-catalogue-binding','official_runtime.py',"if key not in seen or byid[key].get('version')!=version or byid[key]['package']!='nvidia_driver':",'if False:'),
 ('minimal-fetcher-environment','official_runtime.py',"env={k:os.environ[k]for k in ('PATH','SYSTEMROOT','WINDIR','TMPDIR','TEMP','TMP','SSL_CERT_FILE','SSL_CERT_DIR','LANG','LC_ALL','TZ')if k in os.environ}",'env=dict(os.environ)'),
 ('transient-cleanup','official_runtime.py',"with tempfile.TemporaryDirectory(prefix='kf-official-run-')as tmp:","with __import__('contextlib').nullcontext(tempfile.mkdtemp(prefix='kf-official-run-'))as tmp:"),
 ('transient-volume','official_runtime.py',"write_image(files,iso,label='KFAPPSP')","write_image(files,iso,label='KFAPPS')"),
 ('transient-original-bytes','official_runtime.py',"if any(actual[name]!=pin for name,pin in expected.items()):raise MediaDrift('MEDIA_DRIFT original changed during ISO assembly')",'if False:raise MediaDrift()'),
 ('installed-driver-precondition','winapps.py','if c.official_driver and self.driver_version != c.official_driver:','if False:'),
]
def main():
 M.COLLECT="import io,json,unittest\ns=unittest.TestSuite()\nfor p in ('test_official_runtime.py','test_official_harness.py'):s.addTests(unittest.defaultTestLoader.discover('tests',pattern=p))\nr=unittest.TextTestRunner(stream=io.StringIO()).run(s)\nprint(json.dumps({'tests':r.testsRun,'killers':[t.id()for t,_ in r.failures+r.errors]}))"
 with tempfile.TemporaryDirectory(prefix='kf-official-mutants-')as parent:
  baseline=M.run(M.fixture(parent,'baseline'))
  if baseline['killers']:raise RuntimeError('BASELINE RED '+str(baseline['killers']))
  with concurrent.futures.ThreadPoolExecutor(max_workers=2)as pool:rows=list(pool.map(lambda c:M.one(parent,c),CASES))
 print(json.dumps(dict(tests=baseline['tests'],cases=rows),indent=2));return int(any(c['count']<2 for c in rows))
if __name__=='__main__':sys.exit(main())
