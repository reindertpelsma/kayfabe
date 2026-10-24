#!/usr/bin/env python3
"""Hold the GPU lock around the pinned K1 campaign; all host reads are bounded."""
import datetime as dt,hashlib,json,os,re,signal,subprocess,sys,time
from pathlib import Path
import campaign as C
import epoch as E
import native_xid_guard as G
from kayfabe_arm import reload_nvidia
_original_cuda=C.ensure_cuda
def reload_once():
    with (Path('/data/claude-spec-20261024/lane')/'bisect-nvidia-reload-used').open('x') as f:f.write('authorized reload\n')
    reload_nvidia()
def checked_cuda(**kwargs):
    return _original_cuda(reload=reload_once,**kwargs)
C.ensure_cuda=checked_cuda
R=Path('/data/claude-spec-20261024/lane');N=int(sys.argv[1]);REV=sys.argv[2];ROOT=C.arm(N)
S=R/'arms'/str(N);S.mkdir(parents=True,exist_ok=True)
E.pin_binary(R/'kf3-bins'/REV[:8]/'qemu-system-x86_64',REV)

def epoch_record():
    lines=E.read_xids();raw=E._capture(['dmesg']);assert raw.returncode==0
    text=raw.stdout.decode('utf-8','strict');assert len(text.splitlines())>=2
    dm=tuple(x for x in text.splitlines() if 'NVRM: Xid' in x)
    record={'classification':'MEASURED','time':dt.datetime.now().astimezone().isoformat(),'journal_history':lines,'journal_count':len(lines),'journal_sha256':hashlib.sha256('\n'.join(lines).encode()).hexdigest(),'dmesg_count':len(dm),'dmesg_sha256':hashlib.sha256('\n'.join(dm).encode()).hexdigest()}
    (ROOT/'xid-epoch.json').write_text(json.dumps(record,indent=2)+'\n');return lines

owned_helpers={}
history=None
pid=None
from quiet import Quiet,load
quiet=None
try:
    before={int(x.name) for x in Path('/proc').iterdir() if x.name.isdigit()}
    proof=json.loads((R/f'tests-{REV[:8]}.json').read_text());assert proof['rev']==REV and proof['failed']==0
    assert subprocess.run(['pgrep','-x','qemu-system-x86'],stdout=subprocess.DEVNULL).returncode==1,'ANOTHER_VM_RUNNING'
    assert subprocess.check_output(['git','-C',str(R/'srcs'/REV[:8]),'rev-parse','HEAD'],text=True).strip()==REV
    prior_epoch=R/'windows-20261022/boundary-kayfabe-4630/xid-epoch.json'
    if prior_epoch.exists():
        history=json.loads(prior_epoch.read_text())['journal_history']
        G.check_unchanged('\n'.join(E.read_xids()),history)
    C.prepare(N);history=epoch_record();quiet=Quiet(f'windows-{N}').enter();load(S/'load-before-vm.json',f'windows-{N}');C.launch(N,REV[:8])
    state=R/'windows-20261022'/f'windows-broker-run{N}.state'
    pid=int(re.search(r'^QPID=(\d+)$',state.read_text(),re.M)[1]);identity=G.process_identity(pid);assert identity
    for p in Path('/proc').iterdir():
        if not p.name.isdigit() or int(p.name) in before:continue
        try:cmd=(p/'cmdline').read_bytes().replace(b'\0',b' ').decode('utf-8','replace')
        except (FileNotFoundError,ProcessLookupError):continue
        if (str(ROOT) in cmd or ('/data/claude-spec-20261024/lane/production-broker.sh' in cmd and f'stop {N}' in cmd)) and int(p.name)!=os.getpid():
            found=G.process_identity(int(p.name))
            if found:owned_helpers[int(p.name)]=found[1]
    # Include the owned watchdog shells' children while parent identities are live.
    for _ in range(4):
        for item in Path("/proc").iterdir():
            if not item.name.isdigit() or int(item.name) in before:
                continue
            try:
                ppid = int((item / "stat").read_text().rsplit(")", 1)[1].split()[1])
            except (FileNotFoundError, ProcessLookupError):
                continue
            if ppid in owned_helpers:
                found = G.process_identity(int(item.name))
                if found:
                    owned_helpers[int(item.name)] = found[1]
    (ROOT/'session-owned.json').write_text(json.dumps({'controller':os.getpid(),'qemu_pid':pid,'qemu_starttime':identity[1],'owned_helpers':owned_helpers},indent=2)+'\n')
    print('Q4_6D_WINDOWS_LAUNCHED',pid,flush=True)
    resets=[];offset=0
    while True:
        now=time.time();lines=E.read_xids();fresh=E.delta('\n'.join(lines),history)
        with (ROOT/'qemu.log').open() as f:f.seek(offset);chunk=f.read(16*1024*1024+1);offset=f.tell()
        assert len(chunk.encode())<=16*1024*1024,'QEMU observation byte bound'
        resets.extend([now]*C.unload_markers(chunk))
        reason=C.abort_reason(len(fresh),resets,now,C.DEADLINE)
        if (ROOT/'finish.flag').exists() and reason is None:reason='COMPLETE_REQUESTED'
        if G.process_identity(pid) is None:reason='QEMU_EXIT'
        status={'classification':'MEASURED','time':dt.datetime.now().astimezone().isoformat(),'new_host_xids':len(fresh),'rm_unloads_15min':sum(now-900<=x<=now for x in resets),'reason':reason,'qemu_pid':pid}
        (ROOT/'monitor-state.json').write_text(json.dumps(status,indent=2)+'\n')
        if fresh:(ROOT/'new-xids.json').write_text(json.dumps(fresh,indent=2)+'\n')
        if reason:
            (ROOT/'arm-stop.json').write_text(json.dumps(status,indent=2)+'\n');print('Q4_6D_WINDOWS_STOP',reason,flush=True);break
        time.sleep(1)
finally:
    try:
        if (ROOT/'command.json').exists():
            try:C.stop(N)
            finally:
                for pid,start in owned_helpers.items():
                    current=G.process_identity(pid)
                    if current is not None and current[1]==start:
                        try:os.kill(pid,signal.SIGTERM)
                        except ProcessLookupError:pass
        health=C.ensure_cuda(probe=lambda:int(subprocess.check_output(['python3','-c',"import ctypes;print(ctypes.CDLL('libcuda.so.1').cuInit(0))"],text=True)))
        gdm=subprocess.run(['systemctl','is-active','gdm'],capture_output=True,text=True).stdout.strip()
        if gdm!='active':subprocess.run(['systemctl','start','gdm'],check=True);gdm=subprocess.check_output(['systemctl','is-active','gdm'],text=True).strip()
        bound=any((p/'driver').is_symlink() and (p/'driver').resolve().name=='nvidia' for p in Path('/sys/bus/pci/devices').iterdir())
        smi=subprocess.run(['nvidia-smi','--query-gpu=name,driver_version','--format=csv,noheader'],capture_output=True,text=True,timeout=15)
        result={'classification':'MEASURED','time':dt.datetime.now().astimezone().isoformat(),'cuda':health,'gdm':gdm,'nvidia_bound':bound,'smi_exit':smi.returncode}
        (S/'restoration.json').write_text(json.dumps(result,indent=2)+'\n')
        assert health['after']==0 and gdm=='active' and bound and smi.returncode==0
        final_fresh=E.delta('\n'.join(E.read_xids()),history) if history is not None else ()
        if final_fresh:
            ((ROOT/'arm-stop.json') if ROOT.exists() else (S/'safety-event.json')).write_text(json.dumps({'classification':'MEASURED','time':dt.datetime.now().astimezone().isoformat(),'new_host_xids':len(final_fresh),'reason':'NEW_HOST_XID_AFTER_STOP','qemu_pid':pid})+'\n')
            raise RuntimeError('NEW_HOST_XID_AFTER_STOP')
        (S/'session-exit.json').write_text(json.dumps({'classification':'MEASURED','time':dt.datetime.now().astimezone().isoformat(),'run':N,'session_exit':True},indent=2)+'\n')

    finally:
        if quiet is not None:quiet.close()
