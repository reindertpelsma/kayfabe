import hashlib,importlib.util,json,os,re,signal,subprocess,time
def terminate(signum,frame): raise SystemExit(128+signum)
signal.signal(signal.SIGTERM,terminate)
from pathlib import Path
repo=Path('/root/kayfabe');work=Path('/root/ada2build590-validation');work.mkdir(exist_ok=False)
rev=subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip();assert rev.startswith('9d6f210b')
binary=Path('/workspace/bench/kf3-bins/9d6f210b/qemu-system-x86_64');monitor=str(work/'qemu.mon')
spec=importlib.util.spec_from_file_location('owned',repo/'scripts/bench/display/owned_qemu.py');owned=importlib.util.module_from_spec(spec);spec.loader.exec_module(owned)
host=subprocess.check_output(['nvidia-smi','--query-gpu=driver_version','--format=csv,noheader'],text=True).strip();assert host=='595.91.07'
parent=Path('/workspace/bench/guest-590.48.01.qcow2');assert parent.is_file() and Path(str(parent)+'.STAGED').is_file()
image=work/'headless.qcow2';subprocess.run(['qemu-img','create','-q','-f','qcow2','-F','qcow2','-b',str(parent),str(image)],check=True)
assert not subprocess.run(['pgrep','-x','qemu-system-x86'],stdout=subprocess.DEVNULL).returncode==0
args=[str(binary),'-name','kayfabe-interactive','-object','memory-backend-memfd,id=ram0,size=8192M,share=on','-machine','q35,accel=kvm,memory-backend=ram0','-m','8192','-cpu','host','-smp','8','-drive',f'if=virtio,file={image},format=qcow2','-netdev','tap,id=n0,ifname=nvktap0,script=no,downscript=no','-device','virtio-net-pci,netdev=n0,mac=52:54:00:12:34:56,romfile=','-vga','none','-device','kf3-gpu,id=kf0,guest-driver=590.48.01,fb-mb=4096,bar1-size=134217728,bar2-size=33554432,display=off','-display','none','-msg','timestamp=on','-serial',f'file:{work}/serial.log','-monitor',f'unix:{monitor},server,nowait']
with (work/'native-cuda.log').open('w') as out:subprocess.run(['bash',str(repo/'scripts/bench/cuda_ladder.sh'),'host','codex_ada2build_native_for590','1'],stdout=out,stderr=subprocess.STDOUT,check=True,timeout=300)
assert Path('/workspace/bench/cl_codex_ada2build_native_for590_host.out').read_text().count('verdict=PASS')==4
start=time.time();print('HEADLESS590_START',rev,time.strftime('%FT%TZ',time.gmtime()),flush=True)
env={k:v for k,v in os.environ.items() if not k.startswith('KF3_')}
ssh=['ssh','-i','/workspace/bench/guest_key','-o','BatchMode=yes','-o','StrictHostKeyChecking=no','-o','UserKnownHostsFile=/dev/null','-o','LogLevel=ERROR','-o','ConnectTimeout=5','ubuntu@192.168.77.2']
with (work/'qemu.log').open('w') as log:
 p=subprocess.Popen(args,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT,env=env)
 try:
  owned.record(p.pid,str(binary),monitor,str(work/'owned-receipt.json'))
  procenv={s.split(b'=',1)[0].decode():s.split(b'=',1)[1].decode() for s in Path(f'/proc/{p.pid}/environ').read_bytes().split(b'\0') if s.startswith(b'KF3_')};assert procenv=={}
  (work/'actual-process-receipt.json').write_text(json.dumps({'pid':p.pid,'source_revision':rev,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'actual_KF3_environment':procenv,'host_driver':'595.91.07','guest_driver_expected':'590.48.01','argv':args,'start_epoch':start},indent=2)+'\n')
  for _ in range(60):
   assert p.poll() is None,'QEMU exited before guest ready'
   if subprocess.run(ssh+['true'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=8).returncode==0:break
   time.sleep(3)
  else:raise RuntimeError('guest SSH timeout')
  # Copy only public test sources from this pinned checkout, never captured tables.
  with (work/'sources.tar').open('wb') as bundle:
   subprocess.run(['tar','-C',str(repo),'-cf','-','scripts/bench/cuda_min','scripts/bench/cup3.c','scripts/bench/cup8.c','scripts/bench/cup8bench.c','archive/nvkvm/tests/mode2/cup2.c'],stdout=bundle,check=True)
  with (work/'sources.tar').open('rb') as bundle:subprocess.run(ssh+['mkdir -p ~/codex590; tar -xf - -C ~/codex590'],stdin=bundle,check=True,timeout=60)
  guest=r'''
set -euo pipefail
cd ~/codex590
printf 'ACTUAL_GUEST_DRIVER='; nvidia-smi --query-gpu=driver_version --format=csv,noheader
test "$(nvidia-smi --query-gpu=driver_version --format=csv,noheader)" = 590.48.01
test "$(modinfo -F version nvidia)" = 590.48.01
I=-Iscripts/bench/cuda_min; L=/usr/lib/x86_64-linux-gnu/libcuda.so.1
for rung in cup2 cup3 cup8 cup8bench; do
 case "$rung" in cup2) src=archive/nvkvm/tests/mode2/cup2.c;; *) src=scripts/bench/$rung.c;; esac
 gcc -O2 "$I" -o "$rung" "$src" "$L" -lm
 echo "CUDA_RUNG_START=$rung $(date -u -Is)"
 set +e
 BENCH_SIZES=16,2048 BENCH_ITERS=20 BENCH_BATCH=10 timeout 180 "./$rung" >"$rung.log" 2>&1
 rc=$?
 set -e
 cat "$rung.log"
 echo "CUDA_RUNG_EXIT=$rung rc=$rc $(date -u -Is)"
 test "$rc" = 0
 case "$rung" in
 cup2) grep -q 'CE rv=0xabcd1234 want=0xabcd1234 -> PASS' "$rung.log";;
 cup3) grep -q 'KERNEL rv=43 want=43 -> PASS' "$rung.log";;
 cup8) grep -q 'CUP8 RESULT N=2048 bad=0 maxerr=0' "$rung.log";;
 cup8bench) grep -q '^BENCH_TOTAL_BAD=0$' "$rung.log";grep -q '^BENCH_VERDICT: PASS' "$rung.log";;
 esac
 echo "CUDA_CELL rung=$rung verdict=PASS"
done
echo CUDA590_LADDER=PASS
'''
  with (work/'cuda-guest.log').open('w') as out:subprocess.run(ssh+['bash -s'],input=guest,text=True,stdout=out,stderr=subprocess.STDOUT,check=True,timeout=840)
  with (work/'gfx-guest.log').open('w') as out:subprocess.run(ssh+['bash ~/gfx/gfx_steps.sh guest ~/gfxbin'],stdout=out,stderr=subprocess.STDOUT,check=True,timeout=720)
  graphics=(work/'gfx-guest.log').read_text();native=Path('/root/prov/ada2build590_native_gfx.log').read_text()
  assert all('GFX_S'+str(i)+'=PASS' in graphics for i in range(1,6)),'graphics step failed'
  def hashes(text):return dict(re.findall(r'^(VKC_HASH|VKR_HASH_[AZB]|EGLR_HASH_[AZB]|GLXR_HASH_[AZB])=([^\r\n]+)',text,re.M))
  gh,nh=hashes(graphics),hashes(native);assert len(gh)>=10 and gh==nh,(gh,nh)
  print('HEADLESS590_CUDA4_GFX10=PASS',flush=True)
  with (work/'guest-after.log').open('w') as out:subprocess.run(ssh+['sudo dmesg | grep -E "Xid|Flip event timeout" || true; nvidia-smi --query-gpu=driver_version --format=csv,noheader'],stdout=out,check=True,timeout=30)
  after=(work/'guest-after.log').read_text();assert 'Xid' not in after and 'Flip event timeout' not in after
 finally:
  try:subprocess.run(ssh+['sudo poweroff'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=20)
  except subprocess.TimeoutExpired:pass
  try:p.wait(timeout=30)
  except subprocess.TimeoutExpired:
   data=json.loads((work/'owned-receipt.json').read_text());assert owned.stop_process(data,60,15)==0
   p.wait(timeout=15)
  print('HEADLESS590_CLEAN_STOP',p.returncode,time.strftime('%FT%TZ',time.gmtime()),flush=True)
print('HEADLESS590_COMPLETE',time.strftime('%FT%TZ',time.gmtime()),flush=True)
