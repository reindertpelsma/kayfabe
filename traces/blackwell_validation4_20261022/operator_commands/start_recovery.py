from pathlib import Path
import subprocess,json,datetime,os
root=Path('/root/prov')
assert not (root/'XID_STOP').exists()
assert not subprocess.run(['pgrep','-x','qemu-system-x86'],capture_output=True).stdout
p=int((root/'secondary_guard.pid').read_text());os.kill(p,0)
assert 'GUEST_PANICKED' in Path('/workspace/bw-capc-validation/kfwin/logs/on_precision_run.log').read_text()
assert 'ARM_EXIT run=on_precision rc=255' in (root/'arm_on_precision.log').read_text()
subprocess.run(['sha256sum','-c',str(root/'bwr_parent_sha256.txt')],check=True)
log=(root/'manual_collect_on_precision_launcher.log').open('xb')
child=subprocess.Popen(['bash','/root/sweep2/collect_bw.sh','on_precision'],stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
rec={'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'controller_exit':'expected first-enable0x50 closed guestSSH255; no Xid/ownedQEMU remains','recovery_pid':child.pid,'secondary_guard':p,'operation':'approved standardVGA metadata recovery before fixed OFF/B1 arms; no product/exception change'}
(root/'manual_collect_on_precision_launcher.json').write_text(json.dumps(rec,indent=2)+'\n');print(json.dumps(rec))
