"""Owned bench recipe: exactly-once PowerShell launch; status-only reconnects."""
import base64,importlib.util,json,socket,sys,time
from pathlib import Path

run=Path('/workspace/ampere-winvm/ampere580/run')
repo=Path('/root/kayfabe')
spec=importlib.util.spec_from_file_location('appqga',repo/'scripts/bench/windows/appmatrix/qga.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

class BoundedAgent(module.Qga):
    def _recv(self):
        while b'\n' not in self.buf:
            if len(self.buf)>512*1024:raise module.QgaError('reply bound exceeded')
            remaining=deadline-time.monotonic()
            if remaining<=0:raise TimeoutError('program deadline during receive')
            self.sock.settimeout(min(15,remaining))
            chunk=self.sock.recv(min(65536,512*1024+1-len(self.buf)))
            if not chunk:raise module.QgaError('agent closed')
            self.buf+=chunk
        line,self.buf=self.buf.split(b'\n',1)
        if len(line)>512*1024:raise module.QgaError('reply bound exceeded')
        try:reply=json.loads(line)
        except ValueError:return {'_garbage':True}
        return reply if isinstance(reply,dict) else {'_garbage':True}

path=Path(sys.argv[1]);seconds=int(sys.argv[2])
assert 0<seconds<=900 and 0<path.stat().st_size<=128*1024
deadline=time.monotonic()+seconds
agent=BoundedAgent(str(run/'qga.sock'),io_timeout=15)
def connect():
    for attempt in range(4):
        remaining=deadline-time.monotonic()
        if remaining<=0:raise TimeoutError('program deadline exceeded during read-only sync')
        agent.io_timeout=min(15,remaining)
        try:agent.connect();return
        except (OSError,module.QgaError):
            agent.close()
            if attempt==3:raise
            print('READONLY_SYNC_RETRY',attempt+1,flush=True)

try:
    connect()
    encoded=base64.b64encode(path.read_text().encode('utf-16le')).decode()
    result=agent.call('guest-exec',{'path':'powershell.exe','arg':['-NoProfile','-ExecutionPolicy','Bypass','-EncodedCommand',encoded],'capture-output':True},retry=False)
    pid=result['pid'];assert type(pid) is int and 0<pid<2**32
    print('GUEST_PROGRAM_STARTED',path.name,'pid='+str(pid),flush=True)
    while time.monotonic()<deadline:
        try:status=agent.call('guest-exec-status',{'pid':pid},retry=False)
        except (OSError,module.QgaError):connect();continue
        if status.get('exited'):
            for name in ['out-data','err-data']:
                data=base64.b64decode(status.get(name,'') or '',validate=True)
                if len(data)>256*1024:raise RuntimeError('decoded output bound exceeded')
                print(data.decode(errors='replace'),flush=True)
            code=status.get('exitcode');print('GUEST_PROGRAM_EXIT',path.name,'code='+str(code),flush=True)
            sys.exit(0 if code==0 else 1)
        time.sleep(.5)
    raise TimeoutError('outcome pending; do not replay guest program')
finally:agent.close()
