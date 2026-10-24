#!/usr/bin/env python3
import argparse, datetime as dt, importlib.util, json, os, pathlib, re, shutil, subprocess, time
from kayfabe_arm import abort_reason, production_environment, ensure_cuda, unload_markers

ROOT = pathlib.Path('/data/claude-spec-20261024/lane')
W = ROOT / 'windows-20261022'
BROKER = ROOT / 'production-broker.sh'
OWNER = pathlib.Path('/var/lib/kf-windows-20261005/boundary-kayfabe-4501/command.json')
QMP = '/var/lib/kf-windows-20261005/boundary-tools/qmp.py'
DEADLINE = dt.datetime.fromisoformat(os.environ.get('SPEC_DEADLINE','2026-10-24T08:00:00+02:00')).timestamp()


def run(args, **kwargs):
    return subprocess.run(args, check=True, text=True, **kwargs)


def owner():
    return json.loads(OWNER.read_text())


def arm(number):
    return W / f'boundary-kayfabe-{number}'


def prepare(number):
    data = owner(); argv = data['argv']; root = arm(number)
    root.mkdir(parents=True, exist_ok=False)
    W.joinpath('boundary-tools').symlink_to('/var/lib/kf-windows-20261005/boundary-tools') if not W.joinpath('boundary-tools').exists() else None
    W.joinpath('kf3-bins').symlink_to(ROOT / 'kf3-bins') if not W.joinpath('kf3-bins').exists() else None
    baseline = ROOT / 'kf3-bins/89fc8a222'
    if not baseline.exists(): baseline.symlink_to('/var/lib/kf-windows-20261005/kf3-bins/89fc8a222')
    assert BROKER.is_file()  # pinned K1 production copy, copied before launch
    drives = [argv[i+1] for i,x in enumerate(argv[:-1]) if x == '-drive']
    disk = pathlib.Path(next(x for x in drives if 'id=disk0' in x).split('file=',1)[1].split(',')[0]).resolve()
    variables = pathlib.Path(next(x for x in drives if 'if=pflash' in x and 'unit=1' in x).split('file=',1)[1].split(',')[0]).resolve()
    run(['qemu-img','create','-q','-f','qcow2','-F','qcow2','-b',str(disk),str(root/'windows.qcow2')])
    shutil.copyfile(variables, root/'OVMF_VARS.fd')
    record={'disk':str(disk),'size':disk.stat().st_size,'mtime_ns':disk.stat().st_mtime_ns,'variables':str(variables),'time':dt.datetime.now().astimezone().isoformat()}
    (root/'backing-before.json').write_text(json.dumps(record,indent=2)+'\n')
    print('COW_PREPARED',number,flush=True)


def launch(number, rev):
    data=owner(); argv=data['argv']; root=arm(number)
    assert (root/'windows.qcow2').is_file() and not (root/'qemu.log').exists()
    seconds=int(DEADLINE-time.time())
    assert seconds > 0
    env={k:v for k,v in os.environ.items() if not k.startswith('KF3_')}
    env.update(production_environment(data,W,rev,seconds))
    extra=os.environ.get('SPEC_EXTRA_FLAGS','').split()
    if extra: env['WIN_FLAGS']=env['WIN_FLAGS']+' '+' '.join(extra)
    env['BROKER_ARGS']='--stats-line'
    env['WIN_CPU']=argv[argv.index('-cpu')+1]
    device=next(x for x in argv if x.startswith('kf3-gpu,'))
    env['WIN_GPU_UUID']=re.search(r'gpu-uuid=([^,]+)',device).group(1)
    net=next(x for x in argv if x.startswith('virtio-net-pci,'))
    env['WIN_MAC']=re.search(r'mac=([^,]+)',net).group(1)
    env['WIN_NET_RESTRICT']='off'
    extra=argv[argv.index('-uuid'):]
    assert not any(any(c.isspace() for c in x) for x in extra)
    env['WIN_QEMU_EXTRA']=' '.join(extra)
    env['WIN_TITLE']=f'claude-spec SPECviewperf {number} {rev}'
    health=ensure_cuda(probe=lambda:int(subprocess.check_output(['python3','-c',"import ctypes; print(ctypes.CDLL('libcuda.so.1').cuInit(0))"],text=True)))
    (root/'cuda-health.json').write_text(json.dumps(health,indent=2)+'\n')
    cursor=subprocess.check_output(['journalctl','-k','-n','0','--show-cursor','--no-pager'],text=True)
    (root/'journal-cursor.txt').write_text(re.search(r'^-- cursor: (.+)$',cursor,re.M).group(1))
    with (root/'launcher.log').open('w') as f:
        run(['bash',str(BROKER),'run',str(number)],env=env,stdout=f,stderr=subprocess.STDOUT)
    actual=json.loads((root/'command.json').read_text())
    desired={k:v for k,v in data['flags'].items() if k!='KF3_PROF'};desired['KF3_DIAG_PROGRESS']='1000'
    for kv in os.environ.get('SPEC_EXTRA_FLAGS','').split(): k,v=kv.split('=',1); desired[k]=v
    assert actual['flags']==desired,(actual['flags'],desired)
    print((root/'launcher.log').read_text(),flush=True)


def guest(number, script, output=None):
    result=subprocess.run(['python3',QMP,str(arm(number)/'qga.sock'),'qga-exec','powershell.exe','-NoProfile','-ExecutionPolicy','Bypass','-Command',pathlib.Path(script).read_text()],text=True,capture_output=True,timeout=75)
    if output: pathlib.Path(output).write_text(result.stdout)
    else: print(result.stdout,flush=True)
    if result.stderr: print(result.stderr,flush=True)
    if result.returncode: raise RuntimeError(f'QGA_EXIT={result.returncode}')


def monitor(number):
    root=arm(number);resets=[];started=time.time()
    prior=root/'benchmark-host-start.json'
    if prior.exists():offset=json.loads(prior.read_text())['log_offset']
    else:
        offset=(root/'qemu.log').stat().st_size
        prior.write_text(json.dumps({'time':started,'log_offset':offset})+'\n')
    cursor=(root/'journal-cursor.txt').read_text()
    while True:
        now=time.time()
        with (root/'qemu.log').open() as f:
            f.seek(offset);chunk=f.read();offset=f.tell()
        resets.extend([now]*unload_markers(chunk))
        xid=subprocess.run(['journalctl','-k','--after-cursor',cursor,'--grep','NVRM: Xid','--output=json','--no-pager'],text=True,capture_output=True,timeout=15)
        assert xid.returncode in (0,1),(xid.returncode,xid.stderr)
        events=[json.loads(x) for x in xid.stdout.splitlines() if x.strip()]
        reason=abort_reason(len(events),resets,now,DEADLINE)
        state={'time':dt.datetime.now().astimezone().isoformat(),'rm_unload_cycles':len(resets),'cycles_15min':sum(now-900<=x<=now for x in resets),'new_host_xids':len(events),'reason':reason}
        (root/'monitor-state.json').write_text(json.dumps(state,indent=2)+'\n')
        if events:(root/'new-xids.json').write_text(json.dumps(events,indent=2)+'\n')
        print(json.dumps(state),flush=True)
        if reason:
            (root/'arm-stop.json').write_text(json.dumps(state,indent=2)+'\n');return
        statefile=W/f'windows-broker-run{number}.state'
        pid=int(re.search(r'^QPID=(\d+)$',statefile.read_text(),re.M).group(1))
        if not pathlib.Path(f'/proc/{pid}').exists():
            state['reason']='QEMU_EXIT';(root/'arm-stop.json').write_text(json.dumps(state,indent=2)+'\n');return
        time.sleep(10)


def stop(number):
    env=os.environ.copy();env['WIN_DIR']=str(W)
    run(['bash',str(BROKER),'stop',str(number)],env=env)
    before=json.loads((arm(number)/'backing-before.json').read_text());disk=pathlib.Path(before['disk'])
    assert disk.stat().st_mtime_ns==before['mtime_ns'] and disk.stat().st_size==before['size']
    (arm(number)/'backing-after.json').write_text(json.dumps({'classification':'MEASURED','size_unchanged':True,'mtime_unchanged':True})+'\n')


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('action',choices=['prepare','launch','guest','monitor','stop']);p.add_argument('number',type=int);p.add_argument('extra',nargs='*');a=p.parse_args()
    if a.action=='launch':launch(a.number,*a.extra)
    elif a.action=='guest':guest(a.number,*a.extra)
    else:globals()[a.action](a.number)
