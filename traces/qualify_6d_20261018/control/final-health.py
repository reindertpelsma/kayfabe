import ctypes, datetime, json, pathlib, subprocess
from pathlib import Path
R=Path('/data/codex-Q4/round-6d')
def cmd(args):
 p=subprocess.run(args,capture_output=True,text=True,timeout=30)
 return {'exit':p.returncode,'stdout':p.stdout.strip(),'stderr':p.stderr.strip()}
r={'classification':'MEASURED','evidence':'final-health.json','time':datetime.datetime.now().astimezone().isoformat(),'nvidia_smi_before':cmd(['nvidia-smi','--query-gpu=name,driver_version,temperature.gpu,memory.used','--format=csv,noheader'])}
r['cuda_before']=int(subprocess.check_output(['python3','-c',"import ctypes;print(ctypes.CDLL('libcuda.so.1').cuInit(0))"],text=True,timeout=30))
if r['cuda_before']==999:
 with (R/'bisect-nvidia-reload-used').open('x') as f:f.write('owner-authorized one reload\n')
 try:
  subprocess.run(['systemctl','stop','gdm'],check=True,timeout=60)
  subprocess.run(['rmmod','nvidia_uvm','nvidia_drm','nvidia_modeset','nvidia'],check=True,timeout=60)
  subprocess.run(['modprobe','-a','nvidia','nvidia_uvm','nvidia_modeset','nvidia_drm'],check=True,timeout=60)
 finally:subprocess.run(['systemctl','start','gdm'],check=True,timeout=60)
r['cuda_after']=int(subprocess.check_output(['python3','-c',"import ctypes;print(ctypes.CDLL('libcuda.so.1').cuInit(0))"],text=True,timeout=30))
r['nvidia_smi_after']=cmd(['nvidia-smi','--query-gpu=name,driver_version,temperature.gpu,memory.used','--format=csv,noheader'])
r['gdm']=cmd(['systemctl','is-active','gdm'])
if r['gdm']['stdout']!='active':
 subprocess.run(['systemctl','start','gdm'],check=True,timeout=60)
 r['gdm']=cmd(['systemctl','is-active','gdm'])
r['nvidia_bound']=any((p/'driver').is_symlink() and (p/'driver').resolve().name=='nvidia' for p in Path('/sys/bus/pci/devices').iterdir())
import configparser
r['automatic_login']={'configured':False,'enabled':False,'user_present':False}
for conf in [Path('/etc/gdm3/custom.conf'),Path('/etc/gdm/custom.conf'),Path('/etc/gdm3/daemon.conf')]:
 if conf.exists():
  c=configparser.ConfigParser();c.read(conf)
  if c.has_section('daemon'):
   enabled=c.get('daemon','AutomaticLoginEnable',fallback='false').strip().lower() in ('true','yes','1')
   user=bool(c.get('daemon','AutomaticLogin',fallback='').strip())
   if enabled and user:r['automatic_login']={'configured':True,'enabled':True,'user_present':True}
import pwd,time
uid=pwd.getpwnam('ubuntu').pw_uid
def display():
 p=subprocess.run(['runuser','-u','ubuntu','--','env',f'XDG_RUNTIME_DIR=/run/user/{uid}',f'DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/{uid}/bus','/usr/bin/python3',str(R/'host-display.py')],capture_output=True,text=True,timeout=20)
 try:return json.loads(p.stdout)
 except json.JSONDecodeError:return {'query_exit':p.returncode}
r['display_before']=display();r['gdm_restart_for_display']=False
if r['display_before'].get('logical_monitor_count',0)==0:
 subprocess.run(['systemctl','restart','gdm'],check=True,timeout=60)
 r['gdm_restart_for_display']=True
 end=time.monotonic()+60
 while time.monotonic()<end:
  current=display()
  if current.get('logical_monitor_count',0)>0:break
  time.sleep(2)
r['display_after']=display()
r['gdm']=cmd(['systemctl','is-active','gdm'])
r['ubuntu_sessions']=cmd(['loginctl','list-sessions','--no-legend','--no-pager'])
r['ubuntu_logged_in']=any(len(line.split())>2 and line.split()[2]=='ubuntu' for line in r['ubuntu_sessions']['stdout'].splitlines())
# Public evidence retains the required user, not unrelated session/user details.
r.pop('ubuntu_sessions',None)
(R/'final-health.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r))
assert r['cuda_after']==0 and r['nvidia_smi_after']['exit']==0 and r['nvidia_bound'] and r['gdm']['stdout']=='active' and r['automatic_login']['configured'] and r['ubuntu_logged_in'] and r['display_after'].get('logical_monitor_count',0)>0
