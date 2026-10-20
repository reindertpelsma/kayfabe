#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mechanical relay integration coverage, complementary to runtime mutations."""
from pathlib import Path
import re,subprocess
ROOT=Path(__file__).resolve().parents[2];p=ROOT/'crates/kf-qemu/src/userpump.rs'
cases=[('private GET/ref read observations',lambda s:s.replace('prof_private_get','missing_get').replace('prof_reference','missing_ref')),
('real successful PUT observations',lambda s:s.replace('prof_private_put = Some(put)','missing_put = Some(put)')),
('real Wait publication',lambda s:s.replace('p.prof_observe(', 'p.missing_observe(')),
('cold source registration',lambda s:s.replace('register_relay_global','missing_registration')),
('wake reason separation',lambda s:s.replace('Wake::Event','Wake::Demand').replace('Wake::Spin','Wake::Demand').replace('Wake::Backstop','Wake::Demand')),
('distinct lock sites',lambda s:s.replace('Site::RelayStorage','Site::Unknown').replace('Site::RelayMapLock','Site::Unknown'))]
original=p.read_text();out=['MEASURED: source integration mutations; shared runtime semantics tested separately.']
for label,mutate in cases:
 try:
  changed=mutate(original);assert changed!=original;p.write_text(changed)
  r=subprocess.run(['python3','-B','-m','unittest','discover','-s','scripts/bench/prof','-p','test_relay_wiring.py'],cwd=ROOT,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=30)
  kills=set(re.findall(r'^(?:FAIL|ERROR): (\S+)',r.stdout,re.M));out.append(f'{label}: {len(kills)} kills: '+', '.join(sorted(kills)));print(out[-1],flush=True)
  if r.returncode==0 or len(kills)<2:raise SystemExit('CORRECTNESS GATE: insufficient mutation kills')
 finally:p.write_text(original)
(Path(__file__).parent/'relay_wiring_mutations.txt').write_text('\n'.join(out)+'\n')
