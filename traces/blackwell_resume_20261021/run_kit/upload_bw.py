"""Owner lane only: fixed text bundle, bounded SSH transport, hash check, explicit owned controller PID."""
from pathlib import Path
import subprocess,json,tarfile,hashlib,sys,ipaddress,os
w=Path('/workspace/bw-wt');d=Path('/workspace/bw-run');iid=int(sys.argv[1]);src=sys.argv[2];generated=d/str(iid);ledger=json.loads((d/'owned_instances.json').read_text());r=next(x for x in ledger if x['id']==iid);assert r['auto_destroy_verified'];os.kill(r['auto_destroy_pid'],0)
a=json.loads(subprocess.check_output(['vastai','show','instances','--raw'],text=True,stderr=subprocess.DEVNULL));x=next(x for x in a if x['id']==iid);ip=str(ipaddress.ip_address(x['public_ipaddr']));port=int(x['ports']['22/tcp'][0]['HostPort']);alias='bwlane-'+str(iid);conf=d/'ssh.conf';conf.write_text(f'''Host {alias}
 HostName {ip}
 Port {port}
 User root
 IdentityFile /root/.ssh/id_ed25519
 IdentitiesOnly yes
 ForwardAgent no
 BatchMode yes
 ConnectTimeout 8
 ServerAliveInterval 15
 ServerAliveCountMax 3
 HostKeyAlias {alias}
 UserKnownHostsFile /dev/null
 StrictHostKeyChecking no
''')
ssh=['ssh','-F',str(conf),alias]
def remote(cmd,stdin=None,timeout=120):
 return subprocess.run([*ssh,cmd],stdin=stdin,capture_output=True,text=stdin is None,timeout=timeout,check=True)
# Caller first waits for this inventory, under the same bounded transport. A refusal stops deployment.
inv=remote('set -eu; date -u; df -h /; test -c /dev/kvm; echo KVM_PRESENT; test $(df --output=avail -k / | tail -1) -ge 60000000; test ! -e /root/sweep_controller.sh')
print(inv.stdout,flush=True);(d/(str(iid)+'_initial_inventory.txt')).write_text(inv.stdout)
(generated/'lease.json').write_text(json.dumps(r,indent=2)+'\n')
files={'provision_box.sh':w/'scripts/bench/provision_box.sh','sweep_controller.sh':generated/'controller.sh','sweep_windows_media.sh':generated/'media.sh','sweep_windows_install.sh':generated/'install.sh','sweep_native_baseline.sh':generated/'native_baseline.sh','sweep_windows580.sh':generated/'bind.sh'}
for name in ['common.sh','bind_receipt.py','crashdump.ps1','collect_crash.ps1','events.ps1','p19.flags','alloc_headers.py','export.py','lease.json']:files['sweep2/'+name]=generated/name
for name in ['diag580.ps1','enable580_once.ps1','prepare_cold580.ps1','verify_stage580.ps1']:files['sweep2/probes/'+name]=generated/name
files['sweep2/probes/diag_min.ps1']=Path('/workspace/codex/sweep2-20261020/diag_min.ps1')
files['sweep2/arm.sh']=generated/'arm.sh';files['sweep2/collect_bw.sh']=generated/'collect_bw.sh';files['sweep2/lane.sh']=generated/'lane.sh'
for name in ['rpc_timing.py','export_state.py','wer_times.ps1']:files['sweep2/'+name]=Path('/workspace/codex/sweep-win-20261021')/name
BH=w/'scripts/bench/blackwell'
for name in ['dump_header.py','keyline_times.py','xid_policy.py','dump_hdr_hex.ps1']:files['sweep2/'+name]=BH/name
files['sweep2/page_subset.py']=BH/'page_subset.py';files['sweep2/oracle_pr65.tsv']=d/'oracle_pr65.tsv';files['sweep2/native_page.txt']=d/'native_page.txt'
files['prov/sweep2_provision_manifest.json']=generated/'manifest.json'
bundle=d/(str(iid)+'_text_bundle.tar.gz')
with tarfile.open(bundle,'w:gz') as tar:
 for name,path in files.items():
  assert path.is_file() and path.stat().st_size<1024*1024 and '..' not in name.split('/') and not name.startswith('/');tar.add(path,arcname=name,recursive=False)
sha=hashlib.sha256(bundle.read_bytes()).hexdigest();print('BUNDLE',iid,len(files),bundle.stat().st_size,sha,flush=True)
# No SFTP subsystem, no keys/account credentials in the bundle; any transfer timeout is a STOP followed by owned teardown.
with bundle.open('rb') as f:remote('umask 077; cat > /root/sweep_bundle.tar.gz',f)
result=remote(f"set -eu; test $(sha256sum /root/sweep_bundle.tar.gz | cut -d' ' -f1) = {sha}; tar -xzf /root/sweep_bundle.tar.gz -C /root; test ! -f /root/prov/sweep_controller.pid; nohup setsid bash /root/sweep_controller.sh </dev/null >/root/prov/sweep_controller.out 2>&1 & sweep_controller_pid=$!; echo $sweep_controller_pid > /root/prov/sweep_controller.pid; kill -0 $sweep_controller_pid; nohup setsid bash /root/sweep2/lane.sh </dev/null >/root/prov/lane.out 2>&1 & echo $! > /root/prov/lane.pid; echo $sweep_controller_pid",timeout=30)
print('CONTROLLER_PID',result.stdout.strip(),flush=True)
assert result.stdout.strip().isdigit(),'controller PID receipt missing'
