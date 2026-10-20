#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import os,re,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2];p=ROOT/'scripts/bench/prof/h1_export.py'
assert os.environ.get('KF_PROF_H1_DIR')
cases=[('causal coverage stays false', 'full_causal_coverage=False','full_causal_coverage=True'),('interval starts at real start',"ts_ns=checked['start_ns']","ts_ns=checked['end_ns']"),('preserve snapshot context','prof=checked','prof={}')]
original=p.read_text();out=['MEASURED: H1 context-export mutants against pinned analyzer.']
for label,a,b in cases:
 try:
  assert a in original;p.write_text(original.replace(a,b));r=subprocess.run(['python3','-B','-m','unittest','discover','-s','scripts/bench/prof','-p','test_h1.py'],cwd=ROOT,text=True,capture_output=True,timeout=30)
  kills=set(re.findall(r'^(?:FAIL|ERROR): (\S+)',r.stdout+r.stderr,re.M));out.append(f'{label}: {len(kills)} kills: '+', '.join(sorted(kills)));print(out[-1],flush=True)
  if r.returncode==0 or len(kills)<2:raise SystemExit('CORRECTNESS GATE: insufficient mutation kills\n'+r.stderr[-3000:])
 finally:p.write_text(original)
(Path(__file__).parent/'h1_mutations.txt').write_text('\n'.join(out)+'\n')
