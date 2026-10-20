#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Bounded passive-source mutations; every case needs two independent killing tests."""
import os,re,shutil,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2];MODEL=ROOT/'crates/kf-prof/src/relay.rs';RUN=ROOT/'crates/kf-prof/src/runtime.rs';READER=ROOT/'scripts/bench/prof/reader.py'
RUST=['nice','-n','19','cargo','test','-p','kf-prof','--features','prof-inject','--test','relay','-j2','--release']
PY=['python3','-B','-m','unittest','discover','-s','scripts/bench/prof','-p','test_relay_reader.py']
def rep(a,b):
 def mutate(s):
  if a not in s:raise RuntimeError('missing mutation anchor '+a)
  return s.replace(a,b)
 return mutate
def stale(s):return s.replace('w[25] = now;','').replace('        Ok(*w)','        w[25]=now;\n        Ok(*w)')
def infer(s):return s.replace('let (kind, target) = o.wait.words();','let (mut kind, target) = o.wait.words();\n        if kind==1 && (o.reference.unwrap_or(0) as u64>=target || o.private_get.unwrap_or(0) as u64>=target) {kind=0;}')
def budget(s):
 a=s.index('pub fn bounded_record(');b=s.index('/// Channel-birth registration',a)
 return s[:a]+'pub fn bounded_record(row:&mut Value)->io::Result<Vec<u8>> {serde_json::to_vec(row).map_err(raw_error)}\n'+s[b:]
CASES=[
('producer retirement marker',MODEL,rep('self.words[23] = 0;','self.words[23] = 1;'),RUST),
('retirement publication',RUN,rep('if self.gauge.retire(self.engine.now()).is_ok() {','if false {'),RUST),
('retired unanswered ledger',RUN,rep('if v[9] != 0 {','if v[9] != 0 && v[23] != 0 {'),RUST),
('runtime guest wrap',RUN,rep('(p + guest_entries - g) % guest_entries','p.saturating_sub(g) % guest_entries'),RUST),
('guest GET change tracking',MODEL,rep('w[30] = now;','w[30] = 0;'),RUST),
('guest unchanged GET age',MODEL,rep('w[5] & 8 == 0 || w[16] != u64::from(n)','true'),RUST),
('reader guest wrap',READER,rep('(gp+ge-gg)%ge','max(0,gp-gg)%ge'),PY),
('unchanged GET age',MODEL,rep('w[5] & 1 == 0 || w[6] != u64::from(n)','true'),RUST),
('unchanged wait age',MODEL,rep('w[9] != kind || w[10] != target','true'),RUST),
('hardware freshness',MODEL,stale,RUST),
('processing reference independence',MODEL,rep('w[8] = u64::from(n);','w[8] = w[6];'),RUST),
('backwards clock',MODEL,rep('if now < self.words[4] {','if false {'),RUST),
('hardware cannot infer a state transition',MODEL,infer,RUST),
('mapped source geometry',RUN,rep('(LEDGER_BASE + self.slot * SOURCE_BYTES) / 8','(LEDGER_BASE + (self.slot+1) * SOURCE_BYTES) / 8'),RUST),
('registration capacity',RUN,rep('(n < SOURCES as u64).then(|| n + 1)','true.then(|| n + 1)'),RUST),
('registration generation',RUN,rep('slot as u64 + 1, key','1, key'),RUST),
('registration overflow count',RUN,rep('|n| Some(n.saturating_add(1))','|n| Some(n)'),RUST),
('live relay ledger',RUN,rep('if v[9] != 0 {','if false {'),RUST),
('runtime private wrap depth',RUN,rep('(p + entries - g) % entries','p.saturating_sub(g) % entries'),RUST),
('wire payload budget',RUN,budget,RUST),
('reader relay seqlock',READER,rep(' or v[0]&1',''),PY),
('reader validity mask',READER,rep('v[at] if v[6]&mask else None','v[at]'),PY),
('reader index bounds',READER,rep(' and get<entries and put<entries',''),PY),
('reader wrapped depth',READER,rep('(put+entries-get)%entries','max(0,put-get)%entries'),PY),
]
assert os.environ.get('CARGO_TARGET_DIR')=='/workspace/k1-target'
lines=['MEASURED: passive relay/source hand mutations; sources restored after each.']
for label,path,mutate,cmd in CASES:
 original=path.read_text()
 try:
  changed=mutate(original);assert changed!=original;path.write_text(changed)
  if cmd[0]=='nice':
   subprocess.run(['df','-h','/'],check=True,stdout=subprocess.DEVNULL)
   if shutil.disk_usage('/').free<4*1024**3:raise SystemExit('disk guard')
  r=subprocess.run(cmd,cwd=ROOT,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=120)
  pattern=r'^test (\S+) \.\.\. FAILED$' if cmd[0]=='nice' else r'^(?:FAIL|ERROR): (\S+)'
  kills=set(re.findall(pattern,r.stdout,re.M));lines.append(f'{label}: {len(kills)} kills: '+', '.join(sorted(kills)));print(lines[-1],flush=True)
  if r.returncode==0 or len(kills)<2:
   (Path(__file__).parent/'relay_mutations.txt').write_text('\n'.join(lines)+'\n');raise SystemExit('CORRECTNESS GATE: insufficient mutation kills\n'+r.stdout[-5000:])
 finally:path.write_text(original)
(Path(__file__).parent/'relay_mutations.txt').write_text('\n'.join(lines)+'\n')
