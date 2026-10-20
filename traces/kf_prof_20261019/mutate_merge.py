#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import re,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2];M=ROOT/'scripts/bench/prof/merge.py';R=ROOT/'scripts/bench/prof/reader.py'
def rep(a,b):
 def f(s):
  assert a in s,a;return s.replace(a,b)
 return f
cases=[('pacing container bounds',R,lambda s:s.replace('g=g if isinstance(g,dict) else {}','g=g').replace('b=b if isinstance(b,dict) else {}','b=b')),('pacing integer bounds',R,lambda s:s.replace('gap=_count(g,key)','gap=g.get(key)').replace("repeats=_count(b,'repeated')","repeats=b.get('repeated')")),('diag midpoint',M,rep("(selected['host_send_ns']+selected['host_recv_ns'])//2","selected['host_send_ns']")),
('diag uncertainty',M,rep("(selected['host_recv_ns']-selected['host_send_ns']+1)//2","0")),
('no calibration stays unaligned',M,rep('if calibration is None:return out','if False:return out')),
('guest boot identity',M,rep("out['guest_boot_id']!=c['guest_boot_id']","False")),
('calibration validity',M,rep("not c['valid_from_ns']<=stamp<=c['valid_until_ns']","False")),
('measured calibration validation',M,rep('c=normalize_calibration(calibration)','c=calibration')),
('drift uncertainty',M,rep("+(abs(delta)*c['drift_ppm']+999999)//1000000",'+0')),
('bounded guest integers',M,rep('type(v) is int and 0<=v<MAX_NS','type(v) is int')),
('bounded guest lines',M,lambda s:s.replace('4096','32768').replace('4097','32769')),
('fps interval exposure',R,rep("elapsed=row.get('end_ns',0)-row.get('start_ns',0)",'elapsed=1000000000')),
('fps quantile identity',R,rep("low_1pct_fps=low('p99_gap_ns')","low_1pct_fps=low('p999_gap_ns')")),
('missing fps source',R,rep("frames=_count(c,'broker_presented_new_frames')","frames=_count(c,'broker_presented_new_frames') or 0"))]
lines=['MEASURED: clock/merge/pacing helper mutations, all source restored.']
for label,path,mutate in cases:
 original=path.read_text()
 try:
  changed=mutate(original);assert changed!=original;path.write_text(changed)
  pattern='test_merge.py' if path==M else 'test_pacing.py'
  r=subprocess.run(['python3','-B','-m','unittest','discover','-s','scripts/bench/prof','-p',pattern],cwd=ROOT,text=True,capture_output=True,timeout=30)
  kills=set(re.findall(r'^(?:FAIL|ERROR): (\S+)',r.stdout+r.stderr,re.M));lines.append(f'{label}: {len(kills)} kills: '+', '.join(sorted(kills)));print(lines[-1],flush=True)
  if r.returncode==0 or len(kills)<2:raise SystemExit('CORRECTNESS GATE: insufficient mutation kills\n'+r.stderr[-3000:])
 finally:path.write_text(original)
(Path(__file__).parent/'merge_mutations.txt').write_text('\n'.join(lines)+'\n')
