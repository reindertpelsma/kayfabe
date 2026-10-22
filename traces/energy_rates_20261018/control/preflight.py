import ctypes, datetime, json, pathlib, subprocess
from pathlib import Path
R=Path('/data/codex-Q3')
def cmd(args):
 p=subprocess.run(args,capture_output=True,text=True,timeout=30)
 return {'exit':p.returncode,'stdout':p.stdout.strip(),'stderr':p.stderr.strip()}
r={'classification':'MEASURED','evidence':'preflight.json','time':datetime.datetime.now().astimezone().isoformat(),'nvidia_smi_before':cmd(['nvidia-smi','--query-gpu=name,driver_version,temperature.gpu,memory.used','--format=csv,noheader'])}
r['cuda_before']=ctypes.CDLL('libcuda.so.1').cuInit(0)
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
r['nvidia_bound']=any((p/'driver').is_symlink() and (p/'driver').resolve().name=='nvidia' for p in Path('/sys/bus/pci/devices').iterdir())
(R/'preflight.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r))
assert r['cuda_after']==0 and r['nvidia_smi_after']['exit']==0 and r['nvidia_bound'] and r['gdm']['stdout']=='active'
