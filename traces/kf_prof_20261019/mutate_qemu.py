#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutate adapter coverage and lifecycle wiring; shared span semantics tested in Rust."""
import importlib.util
from pathlib import Path
import re,subprocess
ROOT=Path(__file__).resolve().parents[2];path=ROOT/'crates/kf-qemu/src/device.rs'
sp=importlib.util.spec_from_file_location('audit',ROOT/'scripts/bench/prof/wait_audit.py');a=importlib.util.module_from_spec(sp);sp.loader.exec_module(a)
def unwrap(s):
    ts=a.tokens(s);edits=[]
    for i,(t,off) in enumerate(ts):
        if t!='prof_wait' or [w for w,_ in ts[i+1:i+3]]!=['!','(']:continue
        depth=1;j=i+3;comma=None
        while depth:
            w,pos=ts[j]
            if w=='(':depth+=1
            elif w==')':depth-=1
            elif w==',' and depth==1 and comma is None:comma=pos
            j+=1
        edits.append((off,ts[j-1][1]+1,s[comma+1:ts[j-1][1]]))
    assert len(edits)==3
    for lo,hi,new in reversed(edits):s=s[:lo]+new+s[hi:]
    return s
def startup(s):
    return re.sub(r'#\[cfg\(feature = "prof"\)\]\s*let (?:_|_prof_binding) = kf_prof::runtime::(?:init_from_env\(\)|bind_current_global\([123]\));','',s)
def running(s):
    return re.sub(r'#\[cfg\(feature = "prof"\)\]\s*let _prof_running = kf_prof::site\([\s\S]*?\);','',s)
cases=[('adapter poll coverage',unwrap),('optional startup and thread binding',startup),('running residence attribution',running),('distinct drainer wait identities',lambda s:s.replace('QemuDrainerResetPark','QemuVaPark').replace('QemuDrainerPark','QemuVaPark'))]
original=path.read_text();lines=['MEASURED: adapter source-audit mutation checks, not hardware delay injection.']
for label,mutate in cases:
    try:
        changed=mutate(original);assert changed!=original;path.write_text(changed)
        r=subprocess.run(['python3','-B','-m','unittest','discover','-s','scripts/bench/prof','-p','test_qemu_waits.py'],cwd=ROOT,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=60)
        kills=set(re.findall(r'^(?:FAIL|ERROR): (\S+)',r.stdout,re.M))
        lines.append(f'{label}: {len(kills)} kills: '+', '.join(sorted(kills)));print(lines[-1],flush=True)
        if r.returncode==0 or len(kills)<2:raise SystemExit('STOP: mutation refused\n'+r.stdout[-5000:])
    finally:path.write_text(original)
(Path(__file__).parent/'qemu_mutations.txt').write_text('\n'.join(lines)+'\n')
