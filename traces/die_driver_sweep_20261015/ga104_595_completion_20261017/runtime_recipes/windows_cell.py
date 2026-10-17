"""Owned GA104 cell; signed setup launches once, then one cold acceptance boot."""
from pathlib import Path
import datetime, hashlib, json, os, socket, subprocess, time

P = Path('/root/prov')
VM = Path('/workspace/ampere-winvm/ampere580')
SOURCE = '9d6f210ba1dc6ab0edbc49ff265101091e8b08f1'
SCRIPT = '/root/kayfabe/scripts/bench/windows/win_vm.sh'
LOG = open(P/'ampere595_windows_cell.log', 'w', buffering=1)
END = datetime.datetime.fromisoformat('2026-10-17T04:10:00+00:00').timestamp()
FLAGS = '''KF3_WIN_USER_CHANNELS_PASSTHROUGH KF3_WIN_TWIN_DEFAPI_OBJECT KF3_KERNEL_GR_CE KF3_KERNEL_GR_WORK KF3_TRANSLATED_CE_RELAY KF3_DEFERRED_API KF3_PREEMPT_BIND_PROBE KF3_DISPLAY_IMP_ENABLE KF3_DISPLAY_CTRL_PROBE KF3_ZCULL_BIND_PROBE KF3_SW_RUNLIST_HOST_OWNED KF3_ASYNC_PREEMPT KF3_GFX_POOL_PROBE KF3_SW_RUNLIST_PROBE KF3_MEMORY_LIST_PROBE KF3_DISPLAY_SDR_COLOR KF3_DISPLAY_CAPS_PROBE KF3_DISPLAY_LUT_MIRROR KF3_DISPLAY_ILUT_OFFSET_256'''.split()
env = {k:v for k,v in os.environ.items() if not k.startswith('KF3_')}
env.update({k:'1' for k in FLAGS})
env.update(WINVM_ROOT='/workspace/ampere-winvm',WINVM_NAME='ampere580',WINVM_RAM_MB='8192',WINVM_SMP='8',WINVM_KF3_FB_MB='4096',WINVM_QEMU='/root/ampere595_win_qemu.py',SSH_TIMEOUT='20',CARGO_BUILD_JOBS='6',PATH='/root/.cargo/bin:'+os.environ['PATH'])

def note(s):
    print(datetime.datetime.now(datetime.timezone.utc).isoformat(),s,file=LOG,flush=True)

def command(args, seconds=300, output=None, check=True):
    remaining = END-time.time()
    if remaining<=0:raise TimeoutError('owned cell deadline')
    with open(output,'w') if output else open(os.devnull,'w') as f:
        r = subprocess.run(args,env=env,stdout=f if output else LOG,stderr=subprocess.STDOUT,timeout=min(seconds,remaining))
    if check and r.returncode:raise RuntimeError('command failed; inspect recorded result: '+str(output))
    return r.returncode

def ps(name, seconds=300, private=False):
    output=P/('PRIVATE_'+name+'.log' if private else name+'.log')
    note('PS_START '+name)
    rc=command(['python3','/root/ampere59520261017_ps.py','/root/'+name+'.ps1',str(seconds)],seconds+20,output,check=False)
    note('PS_EXIT '+name+' rc='+str(rc))
    if rc:raise RuntimeError('PowerShell result incomplete/failed: '+name+'; never replay installer')
    return output.read_text()

def running():
    p=VM/'run/qemu.pid'
    if not p.is_file():return False
    pid=int(p.read_text())
    q=Path('/proc')/str(pid)
    if not q.exists():return False
    assert str(q.joinpath('exe').resolve())=='/workspace/bench/kf3-bins/9d6f210b/qemu-system-x86_64'
    args=q.joinpath('cmdline').read_bytes().split(b'\0')
    assert any(str(VM/'disk/main.qcow2').encode() in a for a in args)
    return True

own_vm_started=False
def stop():
    if own_vm_started and running():
        note('OWNED_STOP_START')
        # Bound the SSH shutdown attempt; harness then uses ACPI and QMP quit.
        subprocess.run(['bash',SCRIPT,'stop'],env=env,stdout=LOG,stderr=subprocess.STDOUT,timeout=380)
        note('OWNED_STOP_END')

def receipt(tag):
    assert running()
    pid=int((VM/'run/qemu.pid').read_text());q=Path('/proc')/str(pid)
    actual=dict(x.decode().split('=',1) for x in q.joinpath('environ').read_bytes().split(b'\0') if x.startswith(b'KF3_'))
    assert actual=={k:'1' for k in FLAGS},actual
    args=[a.decode() for a in q.joinpath('cmdline').read_bytes().split(b'\0') if a]
    assert any(a.startswith('kf3-gpu,') and 'guest-driver=580.65.06' in a for a in args)
    assert '-vga' in args and args[args.index('-vga')+1]=='none'
    assert '-uuid' in args and args[args.index('-uuid')+1]==(VM/'vm.uuid').read_text().strip()
    binary=q.joinpath('exe').resolve();sha=hashlib.sha256(binary.read_bytes()).hexdigest()
    assert sha=='cc2a86a6dec3ec6bed91d9cd6e1963a1deadf2d75a768e9e3dada91559e34668'
    row=dict(pid=pid,source_revision=SOURCE,binary_sha256=sha,actual_KF3_environment=actual,argv=args,guest_public_abi='580.65.06',expected_windows_build='32.0.15.8088')
    (P/('ampere595_'+tag+'_receipt.json')).write_text(json.dumps(row,indent=2)+'\n')

def ping():
    # Use the bounded runner's read-only sync/ping path, no guest process launch.
    import importlib.util
    spec=importlib.util.spec_from_file_location('appqga','/root/kayfabe/scripts/bench/windows/appmatrix/qga.py')
    m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
    class Bounded(m.Qga):
        def _recv(self):
            while b'\n' not in self.buf:
                if len(self.buf)>32768:raise m.QgaError('ping reply bound')
                chunk=self.sock.recv(min(4096,32769-len(self.buf)))
                if not chunk:raise m.QgaError('closed')
                self.buf+=chunk
            line,self.buf=self.buf.split(b'\n',1)
            if len(line)>32768:raise m.QgaError('ping reply bound')
            try:r=json.loads(line)
            except ValueError:return {'_garbage':True}
            return r if isinstance(r,dict) else {'_garbage':True}
    a=Bounded(str(VM/'run/qga.sock'),io_timeout=5)
    try:a.connect();return a.ping(5)
    except (OSError,m.QgaError):return False
    finally:a.close()

def start(tag):
    global own_vm_started
    assert subprocess.run(['pgrep','-x','qemu-system-x86'],stdout=subprocess.DEVNULL).returncode!=0
    command(['bash',SCRIPT,'run','--overlay','main','--once','--arm','A','--guest-driver','580.65.06','--no-rpc-trace','--tag',tag,'--detach'],120)
    own_vm_started=True
    for _ in range(20):
        if running():break
        time.sleep(1)
    receipt(tag)
    until=min(time.time()+240,END)
    while time.time()<until:
        if not running():raise RuntimeError('guest exited before QGA')
        if ping():note('QGA_READY '+tag);return
        time.sleep(3)
    raise TimeoutError('QGA boot outcome UNVERIFIED; no relaunch during boot')

assert (P/'OWN_SWEEP_ID').read_text().strip()=='56326220'
fd=os.open(P/'ampere595_windows_cell.started',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600);os.close(fd)
note('WINDOWS_CELL_QUEUED M2 hypothesis: verified580.88/GSP1 coldboot reaches Code0/CUDA/D3D/hold; falsifier any nonzero Code, missing desktop, probe failure or TDR')
try:
    while not (VM/'installed').is_file():
        if 'WINDOWS_BASELINE_EXIT rc=' in (P/'ampere595_windows_baseline.log').read_text():raise RuntimeError('baseline failed; no replay')
        if time.time()>=datetime.datetime.fromisoformat('2026-10-17T03:25:00+00:00').timestamp():raise TimeoutError('insufficient acceptance time; UNRUN')
        time.sleep(3)
    note('BASELINE_VERIFIED_STOCK_ONLY')
    assert subprocess.check_output(['git','-C','/root/kayfabe','rev-parse','HEAD'],text=True).strip()==SOURCE
    command(['bash',SCRIPT,'sign'],300,P/'ampere595_gop_sign.log')
    command(['bash','-c',"dmesg | sed -n '/NVRM: Xid/p'"],30,P/'ampere595_windows_host_xids.before.txt')
    start('ampere580_stage')
    # Rearm only this fresh VM's own account. Private rendered command/output never exported.
    password=(VM/'secrets/win_password').read_text().strip().replace("'","''")
    template="""$ErrorActionPreference='Stop'
$wl='HKLM:\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon'
Set-ItemProperty $wl AutoAdminLogon '1'
Set-ItemProperty $wl DefaultUserName 'kf'
Set-ItemProperty $wl DefaultPassword '@PW@'
Set-ItemProperty $wl DefaultDomainName '.'
Remove-ItemProperty $wl AutoLogonCount -ErrorAction SilentlyContinue
$sk='HKLM:\\SYSTEM\\CurrentControlSet\\Services\\nvlddmkm'
if(!(Test-Path $sk)){New-Item $sk -Force|Out-Null}
New-ItemProperty $sk EnableGpuFirmware -PropertyType DWord -Value 1 -Force|Out-Null
Write-Output 'OWN_VM_AUTOLOGON_AND_SERVICE_GSP_SET'
"""
    private=Path('/root/ampere595_private_autologon.ps1')
    fd=os.open(private,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
    with os.fdopen(fd,'w') as f:f.write(template.replace('@PW@',password))
    ps('ampere595_private_autologon',120,private=True)
    ps('ampere59520261017_install58088',600)
    ps('ampere59520261017_bind8088',240)
    stop()
    start('ampere580_accept')
    census=ps('ampere59520261017_census8088',300)
    if '"problem":0' not in census:raise RuntimeError('actual580.88 bound but adapter unhealthy; M2 falsifier fired')
    ps('ampere59520261017_accept8088',600)
    if END-time.time()<1000:raise TimeoutError('short suite complete; 420second hold UNRUN for retirement margin')
    ps('ampere59520261017_hold8088',900)
    command(['python3','/root/ampere59520261017_shot.py'],45,P/'ampere595_windows_shot_summary.json')
    command(['bash','-c',"dmesg | sed -n '/NVRM: Xid/p'"],30,P/'ampere595_windows_host_xids.after.txt')
    assert (P/'ampere595_windows_host_xids.before.txt').read_bytes()==(P/'ampere595_windows_host_xids.after.txt').read_bytes(),'new host Xid'
    note('WINDOWS_CELL_PASS actual8088/GSP1/P19 short+420s hold; pixels independently checked')
except BaseException as e:
    note('WINDOWS_CELL_OUTCOME '+type(e).__name__+' '+str(e))
finally:
    stop()
    note('WINDOWS_CELL_EXIT; inspect actual census and acceptance artifacts')
