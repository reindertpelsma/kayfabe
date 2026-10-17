import datetime,json,subprocess,time
from pathlib import Path

receipt=json.loads(Path('/workspace/codex/gb206phase2-20261016-owned.json').read_text())
assert receipt['id']==56286918
alias='codex-gb206phase2-56286918'
log=Path('/workspace/codex/gb206phase220261016-launch.log')
def note(s):
    line=datetime.datetime.now(datetime.timezone.utc).isoformat()+' '+s
    with log.open('a') as f:f.write(line+'\n')
    print(line,flush=True)
def ssh(command):return subprocess.run(['ssh','-F','/workspace/codex/ssh_config','-o','ForwardAgent=no',alias,command],capture_output=True,text=True,timeout=30)
note('WAIT_START only_own=56286918')
try:
    while time.time()<receipt['created_epoch_conservative']+60*60:
        r=ssh("tail -n 4 /root/prov/gb206phase_linux.log; test ! -e /root/prov/gb206phase_gpu_wave.pid")
        r.check_returncode()
        if 'GB206PHASE_LINUX_EXIT rc=' in r.stdout and 'GB206PHASE_LINUX_EXIT rc=0 ' not in r.stdout:
            raise RuntimeError('setup failed; no GPU wave launched')
        ready='GB206PHASE_BUILD_READY ' in r.stdout
        image='PUSH_INTEGRITY PASS sha256=539aaad2f7280d7fb80fcf67c16297792b7561113c9084978070e9d94ace044b' in Path('/workspace/codex/gb206phase220261016-image-push.log').read_text()
        note('READY build='+str(ready)+' image='+str(image))
        if ready and image:break
        time.sleep(20)
    else:raise TimeoutError('setup exceeded one-hour launch deadline')
    r=ssh("set -e; test ! -e /root/prov/gb206phase_gpu_wave.pid; test -z \"$(pgrep -x qemu-system-x86 || true)\"; test \"$(git -C /root/kayfabe rev-parse HEAD)\" = f8485f6dbe2cdec62c3b1fa69133b5f266801dcc; nohup bash /root/gb206phase_gpu_wave.sh >/root/prov/gb206phase_gpu_wave.wrapper.log 2>&1 </dev/null & p=$!; echo \"$p\" >/root/prov/gb206phase_gpu_wave.pid; echo OWN_GPU_WAVE_PID=$p")
    if r.returncode:raise RuntimeError('launch response uncertain; inspect owned marker/PID; never blindly replay')
    note(r.stdout.strip());note('LAUNCH_EXIT rc=0')
except Exception as e:
    note('ERROR '+type(e).__name__+': '+str(e));note('LAUNCH_EXIT rc=1');raise
