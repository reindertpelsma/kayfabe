import hashlib,json,os,re,sys,time
from pathlib import Path
pid=int(sys.argv[1]);binary=Path(sys.argv[2]);source=sys.argv[3];label=sys.argv[4]
out=Path('/root/prov/ampere595-fast-receipts');out.mkdir(exist_ok=True)
try:
 for _ in range(100):
  try:actual=Path(os.readlink(f'/proc/{pid}/exe'))
  except OSError:time.sleep(.02);continue
  if actual==binary:break
  time.sleep(.02)
 else:raise RuntimeError('actual executable not observed')
 env={s.split(b'=',1)[0].decode():s.split(b'=',1)[1].decode() for s in Path(f'/proc/{pid}/environ').read_bytes().split(b'\0') if s.startswith(b'KF3_')};assert env=={},env
 argv=[s.decode() for s in Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0') if s]
 assert 'kf-fastguest' in argv and any('guest-driver=595.91.07' in s for s in argv)
 cmdline=argv[argv.index('-append')+1];levels=re.findall(r'(?:^|\s)loglevel=(\d+)',cmdline)
 assert levels[-1]==('3' if label=='quiet' else '6'),levels
 assert re.search(r'\b595\.91\.07\b',Path('/proc/driver/nvidia/version').read_text())
 row={'pid':pid,'label':label,'source_revision':source,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'host_driver':'595.91.07','guest_driver':'595.91.07','actual_KF3_environment':env,'effective_requested_loglevel':levels[-1],'argv':argv}
 (out/f'{pid}.json').write_text(json.dumps(row,indent=2)+'\n')
except Exception as e:(out/f'{pid}.FAILED').write_text(type(e).__name__+': '+str(e)+'\n')
