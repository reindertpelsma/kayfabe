import os,signal,time
from pathlib import Path
expected=Path('/usr/bin/qemu-system-x86_64').resolve()
for name in ['prov.pid','gfxprov.pid']:
 p=Path('/workspace/bench')/name
 if not p.is_file():continue
 raw=p.read_text().strip()
 if not raw.isdigit():raise RuntimeError('bad owned stock PID')
 pid=int(raw)
 try:exe=Path(os.readlink(f'/proc/{pid}/exe'))
 except FileNotFoundError:continue
 if exe!=expected:raise RuntimeError('stock PID executable mismatch')
 argv=Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0')
 images=[b'if=virtio,file=/workspace/bench/guest.qcow2,format=qcow2',b'if=virtio,file=/workspace/bench/guest-595.91.07.qcow2,format=qcow2']
 if not any(a in argv for a in images):raise RuntimeError('stock image ownership mismatch')
 if any(b'kf3-gpu' in a for a in argv):raise RuntimeError('GPU VM refused by stock cleanup')
 # Scoped TERM for GPU-free provisioning; no GPU context or SIGKILL.
 if Path(os.readlink(f'/proc/{pid}/exe'))!=expected:raise RuntimeError('stock PID changed')
 os.kill(pid,signal.SIGTERM)
 for _ in range(30):
  if not Path(f'/proc/{pid}').exists():break
  time.sleep(1)
 else:raise RuntimeError('owned GPU-free QEMU did not stop')
 print('OWN_STOCK_STOP_COMPLETE',pid,flush=True)
