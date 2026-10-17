"""One bounded fresh stock-VGA install; never replay an uncertain installer."""
from pathlib import Path
import datetime,json,os,signal,subprocess,time
root=Path('/root/prov')
log=open(root/'ampere595_windows_baseline.log','w',buffering=1)
def note(s):print(datetime.datetime.now(datetime.timezone.utc).isoformat(),s,file=log,flush=True)
def epoch(s):return datetime.datetime.fromisoformat(s).timestamp()
assert (root/'OWN_SWEEP_ID').read_text().strip()=='56326220'
note('WINDOWS_BASELINE_QUEUED')
while True:
    queue=(root/'ampere595_apps_queue.log').read_text()
    if 'AMPERE595_APPS_EXIT rc=' in queue and 'AMPERE595_APPLICATIONS_COMPLETE ' not in queue:
        note('WINDOWS_BASELINE_UNRUN Linux acceptance not complete');raise SystemExit(2)
    if 'AMPERE595_APPLICATIONS_COMPLETE ' in queue and (root/'AMPERE595_LINUX_EVIDENCE_PUSHED').is_file():break
    if time.time()>=epoch('2026-10-17T03:00:00+00:00'):
        note('WINDOWS_BASELINE_UNRUN insufficient lease time');raise SystemExit(2)
    time.sleep(3)
assert subprocess.run(['pgrep','-x','qemu-system-x86'],stdout=subprocess.DEVNULL).returncode!=0
assert 'WINDOWS_FETCH_EXIT rc=0 ' in (root/'ampere595_windows_fetch.log').read_text()
marker=root/'ampere595_windows_baseline.started'
fd=os.open(marker,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600);os.close(fd)
env=os.environ.copy();env.update(WINVM_ROOT='/workspace/ampere-winvm',WINVM_NAME='ampere580',WINVM_RAM_MB='8192',WINVM_SMP='8',WINVM_KF3_FB_MB='4096',WINVM_QEMU='/root/ampere595_win_qemu.py')
script='/root/kayfabe/scripts/bench/windows/win_vm.sh'
status=2
p=None
try:
    note('WINDOWS_BASELINE_DEPENDENCIES_START')
    deps=subprocess.run(['apt-get','-o','DPkg::Lock::Timeout=300','install','-y','-qq','swtpm','swtpm-tools','sbsigntool','python3-virt-firmware','xorriso','netpbm'],stdout=log,stderr=subprocess.STDOUT,env={**env,'DEBIAN_FRONTEND':'noninteractive'},timeout=600)
    if deps.returncode:raise RuntimeError('Windows dependencies failed before install')
    note('WINDOWS_BASELINE_INSTALL_START no GPU device; LTSC media hash pinned')
    p=subprocess.Popen(['bash',script,'install'],stdout=log,stderr=subprocess.STDOUT,env=env,start_new_session=True)
    (root/'ampere595_windows_baseline.install.pid').write_text(str(p.pid)+'\n')
    deadline=min(time.time()+3600,epoch('2026-10-17T03:50:00+00:00'))
    while p.poll() is None:
        if time.time()>=deadline:raise TimeoutError('stock Windows install timebox')
        time.sleep(3)
    if p.returncode:raise RuntimeError('stock Windows install exited nonzero')
    assert Path('/workspace/ampere-winvm/ampere580/installed').is_file()
    assert subprocess.run(['pgrep','-x','qemu-system-x86'],stdout=subprocess.DEVNULL).returncode!=0
    status=0;note('WINDOWS_BASELINE_INSTALLED stock VGA only; NVIDIA binding not yet tested')
except BaseException as e:
    note('BASELINE_ERROR '+type(e).__name__+' '+str(e))
    # This is this VM's own stop path: guest shutdown, ACPI, then QMP quit. Never SIGKILL.
    subprocess.run(['bash',script,'stop'],stdout=log,stderr=subprocess.STDOUT,env=env,timeout=360)
    if p is not None and p.poll() is None:
        os.kill(p.pid,signal.SIGTERM)
        p.wait(timeout=30)
finally:
    note('WINDOWS_BASELINE_EXIT rc='+str(status))
raise SystemExit(status)
