"""Run only GPU-free synthetic tests; restore each source before the next mutant."""
from pathlib import Path
import hashlib
import json
import os
import re
import subprocess
import time

root=Path(__file__).resolve().parents[3]
H='crates/kf-host/src/precomp.rs';L='crates/kf-host/src/precomp_layout.rs';D='crates/kf-disp/src/hostcaps.rs';C='crates/kf-disp/src/capsderive.rs'
original={p:(root/p).read_text() for p in [H,L,D,C]}
def replace(old,new):
    def apply(s):
        assert s.count(old)==1,(old,s.count(old));return s.replace(old,new)
    return apply
mutants=[
 ('write_instead_of_read',H,replace('op[l.operation] = l.read32;','op[l.operation] = l.read32.wrapping_add(1);')),
 ('permit_administrator_bypass',H,replace('if !unprivileged {','if false {')),
 ('ignore_per_register_denial',H,replace('if op[l.status] != l.success {','if false {')),
 ('ignore_changed_register_metadata',H,replace('&& got != want','&& false')),
 ('return_partial_page_on_ioctl_failure',H,replace('control(l.cmd, &mut params, &mut ops)?;','if control(l.cmd, &mut params, &mut ops).is_err() { return Ok(result); }')),
 ('remove_bounded_plan_checks',H,replace('addresses.is_empty()\n        || addresses.len() > MAX_WORDS\n        || ',' ')),
 ('use_fixed_wrong_register_address',H,replace('&address.to_le_bytes()','&0_u32.to_le_bytes()')),
 ('guess_an_uncompiled_host_abi',L,replace('_ => None,','_ => for_version("595.91.07"),')),
 ('discard_successful_host_snapshot',D,replace('.then_some(self.words.as_slice())','.then_some(self.words.as_slice()).filter(|_| false)')),
 ('or_t1_bits_into_host_values',C,replace('words.insert(offset, value);','words.insert(offset, value | words.get(&offset).copied().unwrap_or_default());')),
 ('hide_host_provenance',C,replace('                    Tier::Host,','                    Tier::Rule,')),
 ('reuse_another_die',D,replace('self.die == die','true')),
 ('reuse_different_virtual_window_count',D,replace('self.windows == row.windows','true')),
 ('ignore_changed_public_field_layout',D,replace('self.layout == table.rows_under(row.classes.caps, PREFIX)','true')),
 ('accept_wrong_length_snapshot',D,replace('values.len() != addresses.len()','false')),
 ('publish_empty_or_inaccessible_snapshot',D,replace('values.iter().all(|v| *v == 0)\n        || values.iter().all(|v| *v == u32::MAX)','false')),
]
env=dict(os.environ,CARGO_TARGET_DIR='/workspace/bw-host-precomp-target')
rows=[]
try:
    for name,path,mutate in mutants:
        target=root/path;target.write_text(mutate(original[path]))
        crate,test=('kf-host','precomp_read') if path in [H,L] else ('kf-disp','host_precomp')
        start=time.monotonic()
        run=subprocess.run(['nice','-n','19','cargo','test','--release','-j2','-p',crate,'--test',test,'--','--test-threads=1'],cwd=root,env=env,capture_output=True,text=True,timeout=180)
        log=run.stdout+'\n'+run.stderr
        target.write_text(original[path])
        assert 'error[E' not in log and 'could not compile' not in log,(name,log[-4000:])
        killers=sorted(set(re.findall(r'^test (.+?) \.\.\. FAILED$',run.stdout,re.M)))
        assert run.returncode!=0 and len(killers)>=2,(name,run.returncode,killers,log[-3000:])
        rows.append({'mutant':name,'killers':killers,'seconds':round(time.monotonic()-start,3)})
        print(name,len(killers),'killers',flush=True)
finally:
    for path,source in original.items():(root/path).write_text(source)
result={'mutants':rows,'restored_source_sha256':{p:hashlib.sha256((root/p).read_bytes()).hexdigest() for p in original}}
Path(__file__).with_name('mutations.json').write_text(json.dumps(result,indent=2)+'\n')
