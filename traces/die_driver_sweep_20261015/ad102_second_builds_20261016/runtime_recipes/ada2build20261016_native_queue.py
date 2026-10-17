import datetime,ipaddress,json,re,subprocess,time
from pathlib import Path
root=Path('/workspace/codex');cfg=root/'ssh_config';own=56302930
receipt=json.loads((root/'ada2build-20261016-owned.json').read_text());assert receipt['id']==own
log=root/'ada2build20261016-native-queue.log'
def note(s):
 with log.open('a') as f:f.write(datetime.datetime.now(datetime.timezone.utc).isoformat()+' '+s+'\n')
def run(args,timeout=60):return subprocess.run(args,capture_output=True,text=True,timeout=timeout)
def ssh(cmd,timeout=30):return run(['ssh','-F',str(cfg),'-o','ForwardAgent=no','codex-ada2build-56302930',cmd],timeout)
rc=1;note('START own56302930; Linux590 evidence already pushed; native581 budget until00:30UTC; originaltimerunchanged')
try:
 r=ssh("set -e; grep '^BROKER_DESKTOP_VERDICT=PASS host595.91.07 guest590.48.01' /root/prov/ada590_broker_x11_validating_v2.log; grep '^EXIT rc=0 ' /root/prov/ada590_broker_x11_validating_v2.log; grep '^FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30' /workspace/bench/codex_ada2build_full590_suite.out")
 if r.returncode:raise RuntimeError('full Linux suite not complete; no disk reimage')
 r=ssh("python3 -c 'import os;from pathlib import Path;xs=[]\nfor p in Path(\"/proc\").iterdir():\n if p.name.isdigit():\n  try:e=os.readlink(p/\"exe\")\n  except OSError:continue\n  if e.endswith(\"qemu-system-x86_64\"):xs.append(int(p.name))\nprint(\"OWN_QEMU_PIDS\",xs);assert not xs'")
 if r.returncode:raise RuntimeError('a QEMU job remains; no reimage')
 rows=json.loads(run(['vastai','show','instances','--raw']).stdout);x=next(z for z in rows if z['id']==own)
 host=str(ipaddress.ip_address(x['public_ipaddr']));port=int(x['ports']['22/tcp'][0]['HostPort'])
 r=run(['scp','-q','-F',str(cfg),'-o','ForwardAgent=no',str(root/'ada2build20261016_native581_install.sh'),'codex-ada2build-56302930:/root/codex_windows_on_vast_install.sh'])
 if r.returncode:raise RuntimeError('installer stage failed')
 r=ssh('bash /root/codex_windows_on_vast_install.sh',60)
 (root/'ada2build20261016-installer-queue.txt').write_text(r.stdout+r.stderr)
 if r.returncode:raise RuntimeError('installer queue failed')
 note('NATIVE_INSTALL_QUEUED=1')
 for _ in range(60):
  r=ssh('cat /var/lib/vast-reinstall/payload/ssh_host_ed25519_key.pub',15);k=r.stdout.strip().split()
  if r.returncode==0 and len(k)>=2 and k[0]=='ssh-ed25519' and re.fullmatch('[A-Za-z0-9+/=]+',k[1]):
   pub=' '.join(k[:2]);known=root/'known_hosts';alias='codex-native581-'+str(own)
   if run(['ssh-keygen','-F',alias,'-f',str(known)]).returncode==0:raise RuntimeError('native host key already exists; inspect before overwrite')
   with known.open('a') as f:f.write(alias+' '+pub+'\n')
   with cfg.open('a') as f:f.write(f'\nHost codex-ada2build-native581-56302930\n  HostName {host}\n  Port {port}\n  User vast\n  ForwardAgent no\n  HostKeyAlias {alias}\n  UserKnownHostsFile {known}\n  StrictHostKeyChecking yes\n')
   (root/'ada2build20261016-native-public-key.txt').write_text(pub+'\n')
   note('NATIVE_PUBLIC_HOSTKEY_PINNED=1');rc=0;break
  time.sleep(5)
 if rc:raise RuntimeError('native host key not preserved before reboot')
except Exception as e:note('ERROR '+type(e).__name__+': '+str(e))
finally:note('EXIT rc='+str(rc))
raise SystemExit(rc)
