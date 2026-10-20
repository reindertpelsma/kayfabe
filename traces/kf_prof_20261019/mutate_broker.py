#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Independent C cases, shared-reader cases and source-wiring mutants."""
import os,re,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2];C=ROOT/'broker/nb_prof.c';W=ROOT/'broker/nvkvm_broker.c';M=ROOT/'broker/Makefile';EXE=Path('/workspace/k1-target/broker-prof-mutant')
def rep(a,b):
 def f(s):
  assert a in s,a;return s.replace(a,b)
 return f
CASES=[('C calibration geometry',C,rep('put64(p->map + 96, 1000000000);','put64(p->map + 96, 0);')),('unobserved feedback is unknown',C,rep('if (p->feedback_known)','if (true)')),('build opt-in mode',M,rep('ifeq ($(PROF),1)','ifeq (0,1)')),('build mode invalidation',M,rep('$(OBJS): .prof-mode','')),('new content versus repeats',C,rep('if (!fresh) {','if (false) {')),
('new content gap domain',C,rep('ts - p->last_new_ns','ts - p->last_present_ns')),
('quantile rank',C,lambda s:s.replace('sorted[(n * 99 + 99) / 100 - 1]','sorted[n - 1]').replace('sorted[(n * 999 + 999) / 1000 - 1]','sorted[n - 1]')),
('overflow count',C,rep('p->frame_dropped = sat(p->frame_dropped);','p->frame_dropped = p->frame_dropped;')),
('saturating counters',C,rep('return n == UINT64_MAX ? n : n + 1;','return n + 1;')),
('exclusive files',C,rep(' | O_EXCL','')),
('record commit sequence',C,rep('seq(p->map + off, even);','seq(p->map + off, even - 1);')),
('declared geometry',C,rep('put64(p->map + 16, FILE_BYTES);','put64(p->map + 16, 0);')),
('accepted receive hook',W,rep('nb_prof_received();','')),
('content identity hook',W,rep('nb_prof_content(present_ns, s->tick.commits_since_present != 0);','')),
('validated feedback hook',W,rep('nb_prof_feedback(', 'missing_feedback(')),
('optional startup hook',W,rep('nb_prof_boot();',''))]
lines=['MEASURED: C pacing/wire and mechanical source integration mutants.']
for label,path,mutate in CASES:
 original=path.read_text()
 try:
  changed=mutate(original);assert changed!=original;path.write_text(changed)
  r=subprocess.run(['nice','-n','19','cc','-D_GNU_SOURCE','-DNB_PROF','-std=gnu11','-Wall','-Wextra','-Ibroker','-o',str(EXE),'broker/test/test_prof.c','broker/nb_prof.c'],cwd=ROOT,capture_output=True,text=True,timeout=30);assert r.returncode==0,r.stderr
  env=dict(os.environ,KF_PROF_C_BIN=str(EXE));output='';failed=False
  for cmd in [['python3','broker/test/test_prof.py',str(EXE)],['python3','-B','-m','unittest','discover','-s','scripts/bench/prof','-p','test_broker*.py']]:
   r=subprocess.run(cmd,cwd=ROOT,env=env,capture_output=True,text=True,timeout=30);output+=r.stdout+r.stderr;failed|=r.returncode!=0
  kills=set(re.findall(r'^(?:FAIL|ERROR): (\S+)',output,re.M));lines.append(f'{label}: {len(kills)} kills: '+', '.join(sorted(kills)));print(lines[-1],flush=True)
  if not failed or len(kills)<2:raise SystemExit('CORRECTNESS GATE: insufficient mutation kills\n'+output[-3000:])
 finally:path.write_text(original)
(Path(__file__).parent/'broker_mutations.txt').write_text('\n'.join(lines)+'\n')
