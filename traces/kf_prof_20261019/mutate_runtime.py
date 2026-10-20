#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Hand mutations for the persistent runtime; no test edits, >=2 kills each."""
import os
from pathlib import Path
import re
import shutil
import subprocess
ROOT=Path(__file__).resolve().parents[2]
RUNTIME=ROOT/'crates/kf-prof/src/runtime.rs'
READER=ROOT/'scripts/bench/prof/reader.py'

def replace(before,after):
    def apply(s):
        if before not in s:raise RuntimeError('mutation anchor missing: '+before)
        return s.replace(before,after)
    return apply

def refusal(name):
    def apply(s):
        start=s.index('pub fn '+name+'(');end=s.index('\n///',start)
        part=s[start:end]
        if 'c.publish();' not in part:raise RuntimeError('refusal publication missing')
        return s[:start]+part.replace('c.publish();','',1)+s[end:]
    return apply

def typed(s):
    a=s.index('    def uint(v):');b=s.index('    return row',a)
    return s[:a]+s[b:]

CARGO=['nice','-n','19','cargo']
RUST=CARGO+['test','-p','kf-prof','--features','prof-inject','--test','runtime','-j2','--release']
SINK=CARGO+['test','-p','kf-prof','--features','prof-inject','--test','sink','-j2','--release']
PYTHON=['python3','-B','-m','unittest','discover','-s','scripts/bench/prof','-p','test_reader.py']
CASES=[
 ('mmap mirror',RUNTIME,replace('let _ = self.map.store_u64(HostOffset::new((i as u64) * 8), v);',''),RUST),
 ('completed guard',RUNTIME,replace('if c.state.leave(token, now) {','if false {'),RUST),
 ('whole-thread transaction',RUNTIME,replace('c.begin();',''),RUST),
 ('ledger close',RUNTIME,replace('if !c.ledger.close(token, now) {','if true {'),RUST),
 ('ledger refusal visibility',RUNTIME,refusal('ledger_open'),RUST),
 ('span refusal visibility',RUNTIME,refusal('site'),RUST),
 ('record commit',RUNTIME,replace('e.words.store(base, record * 2, Ordering::Release);','e.words.store(base, record * 2 - 1, Ordering::Release);'),RUST),
 ('JSONL append',RUNTIME,replace('file.append(&body)','Err::<(),io::Error>(error("mutated sink"))'),RUST),
 ('production clock gate',RUNTIME,replace('if !diagnostic_clock && !prof_clock::counter_available() {','if false {'),RUST),
 ('diagnostic marker',RUNTIME,replace('flags: diagnostic_clock as u64,','flags: 0,'),RUST),
 ('rotation bound',RUNTIME,replace('if self.bytes + line.len() > self.limit {','if false {'),SINK),
 ('foreign symlink preservation',RUNTIME,replace('entry_exists(&previous)?','previous.try_exists()?'),SINK),
 ('initial sink capacity',RUNTIME,replace('if limit == 0 || limit > 32 * 1024 * 1024 {','if false {'),SINK),
 ('wire record capacity',RUNTIME,replace('if line.len() > self.limit || line.len() > RECORD_BYTES - 63 {','if false {'),SINK),
 ('counter read',ROOT/'crates/kf-linux-raw/src/prof_unsafe.rs',replace('Some(t)','Some(0)'),CARGO+['test','-p','kf-linux-raw','--features','prof','--test','prof_clock','-j2','--release']),
 ('reader odd sequence',READER,replace(' or seq&1',''),PYTHON),
 ('reader actual length',READER,replace("        if os.fstat(fd).st_size!=FILE_BYTES:raise InvalidProfile('wrong actual file length')",''),PYTHON),
 ('reader declared geometry',READER,replace("        if declared!=FILE_BYTES:raise InvalidProfile('invalid declared geometry')",''),PYTHON),
 ('reader typed payload',READER,typed,PYTHON),
 ('reader nesting',READER,replace("            if depth>16:raise InvalidProfile('JSON nesting exceeds 16')",''),PYTHON),
 ('frame ratio',READER,replace('n/d if n is not None and d else None','n/(d+1) if n is not None and d else None'),PYTHON),
]

def main():
    if os.environ.get('CARGO_TARGET_DIR')!='/workspace/k1-target':raise SystemExit('set lane-owned CARGO_TARGET_DIR')
    env=dict(os.environ);env.pop('KF_PROF_FIXTURE_BIN',None)
    lines=['MEASURED: runtime hand mutations; each restored before the next case.']
    for label,path,mutate,cmd in CASES:
        original=path.read_text()
        try:
            path.write_text(mutate(original))
            if cmd[:len(CARGO)]==CARGO:
                subprocess.run(['df','-h','/'],check=True,stdout=subprocess.DEVNULL)
                if shutil.disk_usage('/').free<4*1024**3:raise SystemExit('STOP: disk guard')
            result=subprocess.run(cmd,cwd=ROOT,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,timeout=90)
            if cmd[0]=='python3':kills=re.findall(r'^(?:FAIL|ERROR): (\S+)',result.stdout,re.M)
            else:kills=re.findall(r'^test (\S+) \.\.\. FAILED$',result.stdout,re.M)
            if result.returncode==0 or len(set(kills))<2:
                lines.append(f'{label}: REJECTED; kills={kills}')
                (Path(__file__).parent/'runtime_mutations.txt').write_text('\n'.join(lines)+'\n')
                raise SystemExit(f'STOP: {label}: insufficient kills {kills}\n{result.stdout[-5000:]}')
            lines.append(f'{label}: {len(set(kills))} killing tests: '+', '.join(kills));print(lines[-1],flush=True)
        finally:path.write_text(original)
    (Path(__file__).parent/'runtime_mutations.txt').write_text('\n'.join(lines)+'\n')
if __name__=='__main__':main()
