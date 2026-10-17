import hashlib,json,os,subprocess,sys,time
from pathlib import Path
work=Path(sys.argv[1]);run=Path(sys.argv[2]);rev=sys.argv[3]
pid=int((work/'qemu.pid').read_text());binary=Path('/workspace/bench/kf3-bins')/rev/'qemu-system-x86_64'
for _ in range(50):
    if Path(os.readlink(f'/proc/{pid}/exe'))==binary:break
    time.sleep(.02)
else:raise RuntimeError('private broker QEMU not observed')
env={s.split(b'=',1)[0].decode():s.split(b'=',1)[1].decode() for s in Path(f'/proc/{pid}/environ').read_bytes().split(b'\0') if s.startswith(b'KF3_')}
assert env=={'KF3_REV':rev,'KF3_RPC_TRACE':'1'},env
argv=[s.decode() for s in Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0') if s]
assert any('guest-driver=580.159.04,' in a and 'display=on' in a and 'display-broker=' in a for a in argv)
assert any('/root/codex-broker-gb206phase/' in a and a.endswith('.qcow2,format=qcow2') for a in argv)
source=subprocess.check_output(['git','-C','/root/kayfabe','rev-parse','HEAD'],text=True).strip();assert source.startswith(rev)
host=subprocess.check_output(['nvidia-smi','-i','0','--query-gpu=driver_version','--format=csv,noheader'],text=True).strip();assert host=='595.91.07'
row={'pid':pid,'source_revision':source,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'host_driver':host,'guest_public_abi':'580.159.04','actual_KF3_environment':env,'harness_selector_note':'KF3_REV selects the private binary; KF3_RPC_TRACE is a bounded metadata diagnostic.','argv':argv}
(run/'actual-process-receipt.json').write_text(json.dumps(row,indent=2)+'\n')
print('BROKER_ACTUAL_PROCESS_RECEIPT',json.dumps({k:v for k,v in row.items() if k!='argv'}))
