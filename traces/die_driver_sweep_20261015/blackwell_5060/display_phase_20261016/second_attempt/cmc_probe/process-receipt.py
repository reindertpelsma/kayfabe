import hashlib,json,os,subprocess,sys
from pathlib import Path
run=Path(sys.argv[1]);repo=Path(sys.argv[2]);rev=sys.argv[3]
pid=int((run/'qpid').read_text());expected=Path('/workspace/bench/kf3-bins')/rev/'qemu-system-x86_64'
assert Path(os.readlink(f'/proc/{pid}/exe'))==expected
actual={s.split(b'=',1)[0].decode():s.split(b'=',1)[1].decode() for s in Path(f'/proc/{pid}/environ').read_bytes().split(b'\0') if s.startswith(b'KF3_')}
requested=dict(l.split('=',1) for l in (run/'flags.txt').read_text().splitlines() if l.startswith('KF3_'))
assert actual==requested,(actual,requested)
diagnostics={"KF3_RPC_TRACE","KF3_RPC_BODY_TRACE"}
production={"KF3_WIN_USER_CHANNELS_PASSTHROUGH","KF3_WIN_TWIN_DEFAPI_OBJECT","KF3_KERNEL_GR_CE","KF3_KERNEL_GR_WORK","KF3_TRANSLATED_CE_RELAY","KF3_DEFERRED_API","KF3_PREEMPT_BIND_PROBE","KF3_DISPLAY_IMP_ENABLE","KF3_DISPLAY_CTRL_PROBE","KF3_ZCULL_BIND_PROBE","KF3_SW_RUNLIST_HOST_OWNED","KF3_ASYNC_PREEMPT","KF3_GFX_POOL_PROBE","KF3_SW_RUNLIST_PROBE","KF3_MEMORY_LIST_PROBE","KF3_DISPLAY_SDR_COLOR","KF3_DISPLAY_CAPS_PROBE","KF3_DISPLAY_LUT_MIRROR","KF3_DISPLAY_ILUT_OFFSET_256"}
assert set(actual) in [production,production|diagnostics,production|diagnostics|{"KF3_CMC_LOG_BUFFER_PROBE"}] and set(actual.values())=={"1"},actual
argv=[s.decode() for s in Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0') if s]
assert any('guest-driver=580.65.06' in s for s in argv)
host=subprocess.check_output(['nvidia-smi','-i','0','--query-gpu=driver_version','--format=csv,noheader'],text=True).strip();assert host=='595.91.07'
source=subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip();assert source.startswith(rev)
record={'pid':pid,'source_revision':source,'binary_revision':rev,'binary_sha256':hashlib.sha256(expected.read_bytes()).hexdigest(),'host_driver':host,'guest_public_abi':'580.65.06','expected_windows_driver':'32.0.15.8088','actual_KF3_environment':actual,'argv':argv}
(run/f'process-receipt-{pid}.json').write_text(json.dumps(record,indent=2)+'\n')
print('ACTUAL_PROCESS_RECEIPT',json.dumps({k:v for k,v in record.items() if k!='argv'},sort_keys=True))
