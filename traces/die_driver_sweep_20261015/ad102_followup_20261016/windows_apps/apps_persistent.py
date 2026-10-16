import base64,importlib.util,json,socket,subprocess,time,sys
from pathlib import Path
repo=Path('/root/kayfabe');run=Path('/root/win/codex-adaref2')
spec=importlib.util.spec_from_file_location('appqga',repo/'scripts/bench/windows/appmatrix/qga.py');module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

class BoundedAgent(module.Qga):
    def _recv(self):
        while b'\n' not in self.buf:
            if len(self.buf)>16*1024*1024:raise module.QgaError('reply byte bound exceeded')
            try:chunk=self.sock.recv(65536)
            except (socket.timeout,TimeoutError):raise module.QgaTimeout('agent read timeout')
            if not chunk:raise module.QgaError('agent closed')
            self.buf+=chunk
        line,self.buf=self.buf.split(b'\n',1)
        if len(line)>16*1024*1024:raise module.QgaError('reply byte bound exceeded')
        try:return json.loads(line)
        except ValueError:return {'_garbage':True}

agent=BoundedAgent(str(run/'qga.sock'),io_timeout=30)
def connect():
    for attempt in range(4):
        try:agent.connect();return
        except (OSError,module.QgaError):
            agent.close()
            if attempt==3:raise
            print('READONLY_SYNC_RETRY',attempt+1,flush=True)

def powershell(path,timeout):
    encoded=base64.b64encode(Path(path).read_text().encode('utf-16le')).decode()
    # Launch exactly once. Only status reads may reconnect afterward.
    result=agent.call('guest-exec',{'path':'powershell.exe','arg':['-NoProfile','-ExecutionPolicy','Bypass','-EncodedCommand',encoded],'capture-output':True},retry=False)
    pid=result['pid'];assert type(pid) is int and 0<pid<2**32
    print('GUEST_PROGRAM_STARTED',Path(path).name,'pid='+str(pid),flush=True)
    end=time.monotonic()+timeout
    while time.monotonic()<end:
        try:status=agent.call('guest-exec-status',{'pid':pid},retry=False)
        except (OSError,module.QgaError):connect();continue
        if status.get('exited'):
            for name in ['out-data','err-data']:
                text=base64.b64decode(status.get(name,'') or '').decode(errors='replace');print(text,flush=True)
            code=status.get('exitcode');print('GUEST_PROGRAM_EXIT',Path(path).name,'code='+str(code),flush=True)
            if code!=0:raise RuntimeError('guest program failed; no replay')
            return
        time.sleep(.5)
    raise TimeoutError('guest program outcome still pending; no replay')

print('PERSISTENT_APPS_START',time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),flush=True)
try:
    connect()
    powershell('/root/adaref2_accept.ps1',180)
    powershell('/root/adaref2_apps.ps1',600)
    powershell('/root/adaref2_accept.ps1',180)
    print('WINDOWS_SHORT_APPS_ACCEPTANCE_PASS=1',flush=True)
finally:agent.close()
