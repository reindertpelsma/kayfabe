"""One recovery; a durable before-launch marker guards against installer replay."""
from pathlib import Path
import datetime,json,os,subprocess,time

src=Path('/root/ampere59520261017_windows_cell.py').read_text().split("assert (P/'OWN_SWEEP_ID')")[0]
src=src.replace("P/'ampere595_windows_cell.log'","P/'ampere595_windows_recovery.log'")
src=src.replace('2026-10-17T04:10:00+00:00','2026-10-17T04:05:00+00:00')
exec(src)
assert (P/'OWN_SWEEP_ID').read_text().strip()=='56326220'
assert 'WINDOWS_CELL_EXIT;' in (P/'ampere595_windows_cell.log').read_text()
assert subprocess.run(['pgrep','-x','qemu-system-x86'],stdout=subprocess.DEVNULL).returncode!=0
events=(VM/'logs/ampere580_stage_b1_qmp_events.jsonl').read_text()
assert any('SHUTDOWN' in l and 'guest-shutdown' in l for l in events.splitlines()),'need clean shutdown before using marker absence'
fd=os.open(P/'ampere595_windows_recovery.started',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600);os.close(fd)
note('RECOVERY_START H-preinstaller: original setup never launched; falsifier durable before-launch marker or actual8088 binding. No installer replay if either exists. Batch cached-package and required stored-class GSP1 setup; one cold acceptance only.')
try:
    # Stock VGA exposes the saved registry/files without initializing the GPU.
    gpu_env=env.copy()
    env={k:v for k,v in env.items() if not k.startswith('KF3_')}
    command(['bash',SCRIPT,'run','--overlay','main','--once','--arm','A','--no-kf3','--no-rpc-trace','--tag','ampere580_recoverstock','--detach'],120)
    own_vm_started=True
    until=min(time.time()+240,END)
    while time.time()<until:
        if running() and ping():break
        time.sleep(3)
    else:raise TimeoutError('stock recovery QGA UNVERIFIED')
    note('STOCK_RECOVERY_QGA_READY')
    output=ps('ampere59520261017_recovery_inspect',180)
    row=json.loads(next(l.split('=',1)[1] for l in output.splitlines() if l.startswith('RECOVERY_INSPECT_JSON=')))
    (P/'ampere595_recovery_inspect.json').write_text(json.dumps(row,indent=2)+'\n')
    bound=any(r.get('version')=='32.0.15.8088' for r in row['nvidia_stored_devices'])
    need_install=not row['installer_started'] and not bound
    if row['installer_started'] and not bound:raise RuntimeError('setup previously started but binding incomplete; refuse installer replay')
    if need_install:
        assert any(f['name']=='driver58088.exe' and f['bytes']==890807536 for f in row['files']), 'no complete cached package; stop precondition'
        assert any(f['name']=='7zr.exe' and f['bytes']==602624 for f in row['files']), 'archiver missing'
        note('ORIGINAL_NVIDIA_SETUP_UNRUN_PROVEN clean shutdown plus absent before-launch marker and no8088; cached preparation only may be retried')
    else:note('NVIDIA_ALREADY_BOUND no installer replay; only GSP preconditions corrected')
    stop();env=gpu_env
    if need_install:
        start('ampere580_recovered_stage')
        ps('ampere59520261017_install58088_cached',600)
        ps('ampere59520261017_bind8088',240)
        stop()
    start('ampere580_accept_recovered')
    census=ps('ampere59520261017_census8088',300)
    if '"problem":0' not in census:raise RuntimeError('actual8088/GSP1 nonzero adapter code; recovery falsifier fired')
    ps('ampere59520261017_accept8088',600)
    if END-time.time()<1000:raise TimeoutError('short acceptance complete; hold UNRUN for retirement margin')
    ps('ampere59520261017_hold8088',900)
    command(['python3','/root/ampere59520261017_shot.py'],45,P/'ampere595_windows_shot_summary.json')
    note('RECOVERY_WINDOWS_PASS actual8088/GSP1/P19 short+420hold+nonblank')
except BaseException as e:
    note('RECOVERY_OUTCOME '+type(e).__name__+' '+str(e))
finally:
    stop()
    note('RECOVERY_EXIT; one recovery complete, no further boot loop')
