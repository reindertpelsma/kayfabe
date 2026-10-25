from pathlib import Path
import datetime,json,subprocess,sys
sys.path.insert(0,'/root/sweep2/guard/blackwell')
import phase_xid_guard as G
r=Path('/root/prov');native=json.loads((r/'native_receipt.json').read_text())['approved_native_xid_lines']
accepted=json.loads((r/'native_receipt.json.accepted.json').read_text())
allowed=accepted['approved_native_xid_lines'];got=G.N.xid_lines(G.N.kernel_snapshot())
assert len(native)==5 and tuple(allowed[:5])==tuple(native)
assert tuple(got)==tuple(allowed), 'new/unaccepted Xid is fatal'
assert len(allowed)<=9
if len(allowed)>5:
 assert accepted.get('init_utc') and accepted.get('phase')
assert not (r/'XID_STOP').exists()
q=subprocess.run(['pgrep','-x','qemu-system-x86'],capture_output=True,text=True)
assert q.returncode==1
controller=int((r/'sweep_controller.pid').read_text());assert not Path('/proc',str(controller),'exe').exists()
lock=subprocess.run(['flock','-n','/tmp/kayfabe-fastguest.lock','true'],capture_output=True)
assert lock.returncode==0
sources={}
for n,p in [('product','/root/kayfabe'),('readiness','/workspace/bw-readiness'),('native_fixture','/workspace/bw-native-fixture'),('measured','/workspace/bw-measured')]:
 head=subprocess.check_output(['git','-C',p,'rev-parse','HEAD'],text=True).strip()
 status=subprocess.check_output(['git','-C',p,'status','--porcelain','--untracked-files=no'],text=True)
 assert not status
 sources[n]=head
j={'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'native_xid_count':len(native),'total_xid_count':len(got),'exact_guard_accepted_history':True,'XID_STOP':False,'qemu_count':0,'controller_exited':True,'gpu_lock_released':True,'clean_sources':sources,'no_more_product_arms':True}
print(json.dumps(j,indent=2))
