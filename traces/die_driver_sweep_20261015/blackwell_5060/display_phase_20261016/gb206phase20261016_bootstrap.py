from pathlib import Path
import datetime,json,subprocess,time
cfg=Path('/workspace/codex/ssh_config');receipt=json.loads(Path('/workspace/codex/gb206phase-20261016-owned.json').read_text());own=receipt['id'];alias=f'codex-gb206phase-{own}';log=Path('/workspace/codex/gb206phase20261016-bootstrap.log')
def note(s):
 with log.open('a') as f:f.write(datetime.datetime.now(datetime.timezone.utc).isoformat()+' '+s+'\n')
 print(s,flush=True)
def run(args,t=60):return subprocess.run(args,capture_output=True,text=True,timeout=t)
try:
 note(f'START own_id={own}')
 for attempt in range(60):
  if time.time()>receipt['created_epoch_conservative']+15*60:raise RuntimeError('15minute startup deadline exceeded')
  r=run(['vastai','show','instances','--raw']);r.check_returncode();x=next((r for r in json.loads(r.stdout) if r['id']==own),None);assert x is not None,'own rental absent'
  note('STATE '+str(x.get('actual_status'))+' requested='+str(x.get('intended_status')))
  ports=x.get('ports') or {};mapping=ports.get('22/tcp') or []
  ip=x.get('public_ipaddr');port=mapping[0].get('HostPort') if mapping else None
  if ip and port:
   if f'Host {alias}\n' not in cfg.read_text():
    with cfg.open('a') as f:f.write(f'\nHost {alias}\n  HostName {ip}\n  Port {port}\n  User root\n  ForwardAgent no\n  HostKeyAlias codex-initial-{own}\n  UserKnownHostsFile /workspace/codex/known_hosts\n  StrictHostKeyChecking accept-new\n')
   r=run(['ssh','-F',str(cfg),'-o','ConnectTimeout=5','-o','ForwardAgent=no',alias,'set -e; cat /etc/os-release; test -c /dev/kvm; df -h /; nproc; free -m | head -2'],20)
   if r.returncode==0 and 'VERSION_ID="22.04"' in r.stdout:
    Path('/workspace/codex/gb206phase20261016-initial-linux.txt').write_text(r.stdout)
    note('OWN_UBUNTU22_KVM_READY=1');break
  time.sleep(15)
 else:raise RuntimeError('initial SSH startup timebox exceeded')
 r=run(['ssh','-F',str(cfg),'-o','ForwardAgent=no',alias,'set -e; test "$(df -B1 --output=size / | tail -1)" -ge 100000000000; test -c /dev/kvm'],20);r.check_returncode()
 for src,dest in [('/workspace/codex/blackwell-hw/scripts/bench/provision_box.sh','/root/provision_box.sh'),('/workspace/codex/gb206phase20261016_linux.sh','/root/gb206phase_linux.sh')]:
  r=run(['scp','-q','-F',str(cfg),'-o','ForwardAgent=no',src,alias+':'+dest]);r.check_returncode()
 r=run(['ssh','-F',str(cfg),'-o','ForwardAgent=no',alias,'set -e; mkdir -p /root/prov; test ! -e /root/prov/gb206phase_linux.pid; nohup bash /root/gb206phase_linux.sh >/root/prov/gb206phase_linux.wrapper.log 2>&1 </dev/null & p=$!; echo "$p" >/root/prov/gb206phase_linux.pid; echo OWN_LINUX_PID=$p'],20)
 if r.returncode:raise RuntimeError('launch outcome uncertain; inspect owned start marker/PID before any retry')
 note(r.stdout.strip());note('EXIT rc=0')
except Exception as e:
 note('ERROR '+type(e).__name__+': '+str(e));note('EXIT rc=1');raise SystemExit(1)
