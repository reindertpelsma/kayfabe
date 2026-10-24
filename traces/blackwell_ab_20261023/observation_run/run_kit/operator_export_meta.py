from pathlib import Path
import json,subprocess,hashlib,re
D=Path('/workspace/bw3-observe-run')
remote=r"""
from pathlib import Path
import json,subprocess,datetime,hashlib,re
root=Path('/root/prov');out={};total=0
names=['precomp_tier3.txt','precomp_tier3_exit.txt','precomp_tier3_source','precomp_tier3_binary_sha256','readiness_source','readiness.json','readiness.json.arm.json','sweep_20261020_provision.log','sweep_host_die.log','sweep_host_preflight.log','sweep_gates.log','sweep_bare595.log','sweep_cuda_host.log','sweep_native_grtier.log','sweep_qemu_sha256','sweep_product_source','native_fixture_receipt.json','media_cache_receipt.json','audit_gb205_zbc.log','audit_gb205_8162.log','s4_root_table.log','s4_nobody_table.log','precomp_runtime.txt','precomp_exit.txt','XID_STOP','guard_watch.log','native_receipt.json','native_receipt.json.accepted.json','native_processes.jsonl','source_defer-liveness.log','source_missing-page-fault.log','bwr_bind.log','bwr_binding.json','bwr_cold_ready.txt','sweep_windows_media.log','measured_binary.json','measured_build.log','secondary_guard_receipt.json','secondary_guard.log','secondary_receipt.json.accepted.json','pidfd_runtime_smoke.json','b1_merged_build_receipt.json','b1_merged_prepared.json','b1_merged_binary_sha256','b1_merged_unique_binary_sha256','held_final_snapshot.json','bwr_shutdown_census.json','bwr_parent_sha256.txt','native_receipt.json.ready.json','secondary_receipt.json.ready.json','budget_start_on_derived.json','budget_start_on_measured.json','media_wait_continuation.json','sweep_preparation_continuation.log','guard_watch_continued.log','pre_continue_sweep_windows_install.log','arm_history_patch.json','sweep_windows_install.log','observed_b1_binary.json','observation_native_ready.json','budget_start_on_observe_b.json','budget_start_on_observe_b1.json','observed_b1_build.log','observed_b1_tests_summary.json']
for name in names:
 p=root/name
 if not p.exists():continue
 b=p.read_bytes();assert len(b)<=262144;total+=len(b);assert total<=2097152
 s=re.sub(r'GPU-[0-9a-fA-F-]+','GPU-UUID-REMOVED',b.decode('utf-8'))
 out[name]={'source_sha256':hashlib.sha256(b).hexdigest(),'text':s}
p=Path('/workspace/bench/cl_capc_native595_host.out')
if p.exists():
 b=p.read_bytes();assert len(b)<=262144
 out['cuda_primary_rows.out']={'source_sha256':hashlib.sha256(b).hexdigest(),'text':b.decode()}
j=subprocess.check_output(['timeout','10','journalctl','-k','-b','-o','short-iso-precise','--no-pager'],text=True)
x=[l for l in j.splitlines() if 'NVRM: Xid' in l]
out['kernel_xids.json']={'source_sha256':hashlib.sha256(j.encode()).hexdigest(),'text':json.dumps({'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'xid_lines':x},indent=2)+'\n'}
out['precomp_kernel_permission.json']={'source_sha256':hashlib.sha256(j.encode()).hexdigest(),'text':json.dumps({'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'permission_lines':[l for l in j.splitlines() if 'gpuValidateRegOffset' in l or 'does not have permission to access register offset' in l]},indent=2)+'\n'}
for arm in ['on_observe_b','on_observe_b1']:
 p=Path('/workspace/bw-capc-validation/kfwin/logs')/(arm+'_run.log')
 if p.exists():
  b=p.read_bytes();assert len(b)<=32768
  out[arm+'_run.log']={'source_sha256':hashlib.sha256(b).hexdigest(),'text':b.decode()}
print(json.dumps(out))
"""
r=subprocess.run(['ssh','-F',str(D/'ssh.conf'),(D/'alias').read_text().strip(),'python3 -I -'],input=remote,capture_output=True,text=True,timeout=45,check=True)
assert len(r.stdout)<=4194304
out=Path('/workspace/bw-native-edge-wt/traces/blackwell_ab_20261023/observation_run/lease_metadata');out.mkdir(exist_ok=True)
receipts={}
for name,row in json.loads(r.stdout).items():
 assert re.fullmatch('[A-Za-z0-9_.-]+',name)
 text=row['text'];assert not any(s in text for s in [('BEGIN '+'OPENSSH PRIVATE KEY'),'kf-gsp: reply-body '])
 text='\n'.join(l.rstrip() for l in text.splitlines())+'\n';out.joinpath(name).write_text(text)
 receipts[name]={'source_sha256':row['source_sha256'],'export_sha256':hashlib.sha256(text.encode()).hexdigest()}
out.joinpath('export_receipt.json').write_text(json.dumps(receipts,indent=2)+'\n')
print('EXPORTED_META',len(receipts))
