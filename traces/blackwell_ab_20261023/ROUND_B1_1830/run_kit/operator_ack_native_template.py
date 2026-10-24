from pathlib import Path
import json,sys,datetime
sys.path.insert(0,'/root/sweep2/guard/blackwell')
import phase_xid_guard as G
r=Path('/root/prov')
assert not (r/'XID_STOP').exists()
allowed=json.loads((r/'native_receipt.json').read_text())['approved_native_xid_lines']
assert len(allowed)==5 and tuple(G.N.xid_lines(G.N.kernel_snapshot()))==tuple(allowed)
assert 'BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30' in (r/'sweep_bare595.log').read_text()
assert 'GR_TIER_VERDICT=PASS' in (r/'sweep_native_grtier.log').read_text()
rows=[x for x in Path('/workspace/bench/cl_capc_native595_host.out').read_text().splitlines() if x.startswith('CL_ROW ')]
assert len(rows)==4 and all('verdict=PASS ' in x for x in rows)
j={
 'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
 'backup_kind':'bounded text exported to operator workspace before live B',
 'publication_deferred_until_live_complete':True,
 'local_export_receipt_sha256':'__LOCAL_EXPORT_RECEIPT_SHA256__',
 'raw':30,'cuda':4,'gr':15,'native_faults':5,'exact_ordered_fault_match':True,
}
(r/'N0_EXPORT_ACK').write_text(json.dumps(j)+'\n')
print(json.dumps(j))
