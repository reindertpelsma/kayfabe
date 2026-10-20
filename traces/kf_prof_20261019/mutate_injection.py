#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Named delay helper mutation gate. Sources always restored."""
import os,re,shutil,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2];p=ROOT/'crates/kf-prof/src/runtime.rs'
def post(s):
 a='''        c.publish();
        #[cfg(feature = "prof-inject")]
        {
            let request = c.engine.site_delay'''
 b='''        #[cfg(feature = "prof-inject")]
        {
            let request = c.engine.site_delay'''
 assert a in s;s=s.replace(a,b);return s.replace('''        SpanGuard {
            token: Some(token),''','''        c.publish();
        SpanGuard {
            token: Some(token),''')
cases=[('delay duration',lambda s:s.replace('Duration::from_millis(request >> 16)','Duration::ZERO')),
('named target',lambda s:s.replace('request & 0xffff == u64::from(site as u16) + 1','request & 0xffff == u64::from(site as u16) + 2')),
('live publication before delay',post),
('advisory overdue label',lambda s:s.replace('"overdue":age>kind.bound_ns()','"overdue":false').replace('json!(Kind::ALL.get(k).is_some_and(|kind| age > kind.bound_ns()))','json!(false)'))]
assert os.environ.get('CARGO_TARGET_DIR')=='/workspace/k1-target'
original=p.read_text();out=['MEASURED: table-driven injection mutations; restored source after each.']
for label,mutate in cases:
 try:
  assert shutil.disk_usage('/').free>=4*1024**3;changed=mutate(original);assert changed!=original;p.write_text(changed)
  r=subprocess.run(['nice','-n','19','cargo','test','-p','kf-prof','--features','prof-inject','--no-fail-fast','-j2','--release'],cwd=ROOT,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=120)
  kills=set(re.findall(r'^test (\S+) \.\.\. FAILED$',r.stdout,re.M));out.append(f'{label}: {len(kills)} kills: '+', '.join(sorted(kills)));print(out[-1],flush=True)
  if r.returncode==0 or len(kills)<2:raise SystemExit('CORRECTNESS GATE: insufficient mutation kills\n'+r.stdout[-4000:])
 finally:p.write_text(original)
(Path(__file__).parent/'injection_mutations.txt').write_text('\n'.join(out)+'\n')
