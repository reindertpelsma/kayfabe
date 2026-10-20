#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The same two-test gate, scoped to wait-wrapper changes."""
import importlib.util
from pathlib import Path
import os,re,shutil,subprocess
ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('audit',ROOT/'scripts/bench/prof/wait_audit.py');a=importlib.util.module_from_spec(spec);spec.loader.exec_module(a)
def unwrap(s):
    ts=a.tokens(s);edits=[]
    for i,(t,off) in enumerate(ts):
        if t!='prof_wait' or [x[0] for x in ts[i+1:i+3]]!=['!','(']:continue
        depth=1;j=i+3;comma=None
        while depth:
            word,pos=ts[j]
            if word=='(':depth+=1
            elif word==')':depth-=1
            elif word==',' and depth==1 and comma is None:comma=pos
            j+=1
        edits.append((off,ts[j-1][1]+1,s[comma+1:ts[j-1][1]]))
    assert len(edits)==2
    for start,end,new in reversed(edits):s=s[:start]+new+s[end:]
    return s
def lexical(s):
    s='\n'.join(l for l in s.split('\n') if not l.lstrip().startswith(('char=re.match','if char:')))
    s=s.replace("r'(?:r#)?([A-Za-z_][A-Za-z_0-9]*)'", "r'([A-Za-z_][A-Za-z_0-9]*)'")
    s=s.replace("(ts[i-1][0]=='.' or (i>=2 and ts[i-1][0]==ts[i-2][0]==':'))", "ts[i-1][0]=='.'")
    return s
cases=[
('audit lexical boundaries',ROOT/'scripts/bench/prof/wait_audit.py',lexical,['python3','-B','-m','unittest','discover','-s','scripts/bench/prof','-p','test_wait_audit.py']),
('wrapper attribution',ROOT/'crates/kf-prof/src/lib.rs',lambda s:s.replace('let _span = site(at, state, arg);','let _span = ();'),['nice','-n','19','cargo','test','-p','kf-prof','--features','prof-inject','--test','runtime','-j2','--release']),
('audit refusal',ROOT/'scripts/bench/prof/wait_audit.py',lambda s:s.replace('if not (stack and stack[-1]):','if False:'),['python3','-B','-m','unittest','discover','-s','scripts/bench/prof','-p','test_wait_audit.py']),
('channel call-site coverage',ROOT/'crates/kf-chan/src/worker.rs',unwrap,['python3','-B','-m','unittest','discover','-s','scripts/bench/prof','-p','test_wait_audit.py']),
]
assert os.environ.get('CARGO_TARGET_DIR')=='/workspace/k1-target'
lines=['MEASURED: scoped wrapper mutations, sources restored after each.']
for label,path,mutate,cmd in cases:
    original=path.read_text()
    try:
        changed=mutate(original);assert changed!=original;path.write_text(changed)
        if cmd[0]=='nice':
            subprocess.run(['df','-h','/'],check=True)
            if shutil.disk_usage('/').free<4*1024**3:raise SystemExit('STOP disk guard')
        r=subprocess.run(cmd,cwd=ROOT,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,timeout=120)
        pattern=r'^test (\S+) \.\.\. FAILED$' if cmd[0]=='nice' else r'^(?:FAIL|ERROR): (\S+)'
        kills=set(re.findall(pattern,r.stdout,re.M))
        lines.append(f'{label}: {len(kills)} kills: '+', '.join(sorted(kills)));print(lines[-1],flush=True)
        if r.returncode==0 or len(kills)<2:raise SystemExit('STOP: mutation refused\n'+r.stdout[-5000:])
    finally:path.write_text(original)
(Path(__file__).parent/'wrapper_mutations.txt').write_text('\n'.join(lines)+'\n')
