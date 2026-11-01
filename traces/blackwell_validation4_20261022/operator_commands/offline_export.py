from pathlib import Path
import subprocess,json,datetime,hashlib,os
r=Path('/root/prov');run='off_c';folder=r/'arms'/run
assert (r/'XID_STOP').exists()
assert not subprocess.run(['pgrep','-x','qemu-system-x86'],capture_output=True).stdout
q=Path('/workspace/bw-capc-validation/kfwin/logs/off_c_b1_qemu.log')
subprocess.run(['python3','-I','/root/sweep2/export.py',str(q),str(folder)],check=True)
subprocess.run(['python3','-I','/root/sweep2/export_state.py',run],check=True)
subprocess.run(['python3','-I','/root/sweep2/keyline_times.py',str(q),str(folder/'rpc_timing.jsonl'),str(folder/'keylines.jsonl'),'PROBE-DUMP','chan token','BORN ','GSP REFUSED fn76/0x906f0102','GUEST_PANICKED','RC_TRIGGERED','kf3: GSP','UNLOADING'],check=True)
j=subprocess.check_output(['journalctl','-k','-b','-o','short-iso-precise','--no-pager'],text=True)
lines=[x for x in j.splitlines() if 'NVRM: Xid' in x]
rec={'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'own_id':57148807,'product':'8f3a9bb3a5d652b3f536ca015bf95735ec588924','verdict':'ABORTED at guard; not natural TDR measurement or PASS','kernel_xids':lines,'scope':'OFF/GOPoff/emptyROM, no capturedcaps, completionprobe500','code0_before_fault_observed':False,'init_receipt':json.loads((r/'phase.json').read_text()),'canonical_rpc_receipt':json.loads((folder/'rpc_timing.receipt.json').read_text()),'fatal_marker':json.loads((r/'XID_STOP').read_text()),'marker_limitation':'second watcher overwrote first reason with prior fatal marker; original reason not retained','no_qemu':True,'b1':'UNRUN after fatal guard; optional first next authorized lease','no_more_gpu':True}
(folder/'xid_policy.json').write_text(json.dumps(rec,indent=2)+'\n')
subprocess.run(['sha256sum','-c',str(r/'bwr_parent_sha256.txt')],check=True)
(r/'held_final_snapshot.json').write_text(json.dumps({'utc':rec['utc'],'own_id':57148807,'qemu_count':0,'kernel_xid_count':len(lines),'fatal_present':True,'source':rec['product'],'no_more_gpu':True},indent=2)+'\n')
print(json.dumps({'verdict':rec['verdict'],'rpc_receipt':rec['canonical_rpc_receipt'],'xids':len(lines),'qemu_count':0}))
