"""Owned bench recipe: exactly-once PowerShell launch; status-only reconnects."""
import base64,importlib.util,json,socket,sys,time
from pathlib import Path

run=Path('/root/win/codex-ada2build')
repo=Path('/root/kayfabe')
spec=importlib.util.spec_from_file_location('appqga',repo/'scripts/bench/windows/appmatrix/qga.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

class BoundedAgent(module.Qga):
    def _recv(self):
        while b'\n' not in self.buf:
            if len(self.buf)>16*1024*1024:raise module.QgaError('reply bound exceeded')
            chunk=self.sock.recv(65536)
            if not chunk:raise module.QgaError('agent closed')
            self.buf+=chunk
        line,self.buf=self.buf.split(b'\n',1)
        if len(line)>16*1024*1024:raise module.QgaError('reply bound exceeded')
        try:reply=json.loads(line)
        except ValueError:return {'_garbage':True}
        return reply if isinstance(reply,dict) else {'_garbage':True}

pid=int(sys.argv[1]);assert 0<pid<2**32
agent=BoundedAgent(str(run/'qga.sock'),io_timeout=12)
try:
 agent.connect()
 status=agent.call('guest-exec-status',{'pid':pid},retry=False)
 print('RECOVER_ORIGINAL_PID',pid,'exited='+str(status.get('exited')),flush=True)
 if status.get('exited'):
  for name in ['out-data','err-data']:print(base64.b64decode(status.get(name,'') or '').decode(errors='replace'),flush=True)
  print('RECOVERED_EXIT_CODE',status.get('exitcode'),flush=True)
finally:agent.close()
