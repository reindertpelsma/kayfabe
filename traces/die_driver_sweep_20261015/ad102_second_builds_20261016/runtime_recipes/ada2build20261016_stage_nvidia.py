import hashlib,math,subprocess,time
from pathlib import Path
root=Path('/root/ada2build-payload')
helper='/root/kayfabe/scripts/bench/diesweep/qga_file_put.py'
socket='/root/win/codex-ada2build/qga.sock'
source=root/'driver58151.exe';expected='cb04aa1e2fbb977ee3911ab2cf51a42351f8126ba991bc9308ce6bf0ba96e83a'
assert source.stat().st_size==910810544
digest=hashlib.sha256()
with source.open('rb') as stream:
 while chunk:=stream.read(1024*1024):digest.update(chunk)
assert digest.hexdigest()==expected
start=time.monotonic();deadline=start+1200
print('NVIDIA_STAGE_START',time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),flush=True)
try:
 with source.open('rb') as incoming:
  for index in range(math.ceil(source.stat().st_size/(4*1024*1024))):
   if time.monotonic()>=deadline:raise TimeoutError('staging deadline')
   data=incoming.read(4*1024*1024);assert data
   temporary=root/'upload.part';temporary.write_bytes(data)
   destination='C:\\ProgramData\\KayfabeSweep\\downloads\\'+f'nvidia58151.part{index:03d}'
   result=subprocess.run(['python3',helper,socket,str(temporary),destination],capture_output=True,text=True,timeout=min(135,deadline-time.monotonic()))
   if result.returncode:
    print(result.stdout+result.stderr,flush=True);raise RuntimeError(f'chunk{index} outcome uncertain; never automatic replay')
   print(f'NVIDIA_STAGE_CHUNK index={index} bytes={len(data)} '+result.stdout.strip(),flush=True)
 print(f'NVIDIA_STAGE_EXIT rc=0 elapsed={time.monotonic()-start:.3f}',flush=True)
except BaseException:
 print('NVIDIA_STAGE_EXIT rc=1',flush=True);raise
