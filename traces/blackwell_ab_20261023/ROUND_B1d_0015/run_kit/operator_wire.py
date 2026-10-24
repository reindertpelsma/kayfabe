from pathlib import Path
import json,shutil
W=Path('/workspace/bw-native-edge-wt');D=Path('/workspace/bw3-round-b1d-run');T=W/'traces/blackwell_ab_20261023/ROUND_B1d_0015';K=T/'run_kit'
p=D/'ops.py';s=p.read_text().replace('B.plan(float(candidate["dph_total"]), credit, now())','B.plan(float(candidate["dph_total"]), credit, now(), stop_usd=2.0, cap_usd=2.5)').replace('B.plan(rate, credit, now())','B.plan(rate, credit, now(), stop_usd=2.0, cap_usd=2.5)')
s=s.replace('"budget_stop": 2.5','"budget_stop": 2.0').replace('"phase_cap": 3','"phase_cap": 2.5').replace('< 2.5\n                        and row','< 2.0\n                        and row')
s=s.replace('single_market_B1.json','single_market_B1b.json').replace('"codex-bw3-B1-OBSERVATION-20261023"','"codex-bw3-B1b-OBSERVATION-20261023"')
s=s.replace('        57366202,','        57366202,\n        57388945,')
s=s.replace('round stop2.50 cap3 reserve8','round stop2.00 cap2.50 reserve8; B1b native benchmark verdicts are findings')
p.write_text(s)
for n in ['fuzz_fixture.py','fuzz_findings.py','round_budget.py']:
 shutil.copy2(W/'scripts/bench/blackwell'/n,K/n)
p=K/'controller.sh';s=p.read_text()
a='bash scripts/fastguest/bare_metal_suite.sh capc_native595 180 >/root/prov/sweep_bare595.log 2>&1'
args='--timer --engines --doorbell-census --gpu-info-sweep --bus-info-sweep --concurrency --defer-liveness --blockage-coverage --alias-two-vas --alias-unmap-observe --map-propagation --late-map-race --missing-page-fault --uvm-invalidate --uvm-mean --executor-vas --guest-ring-channel --dictated-ring --dictated-ring-negative --ce-client --ce-client-guest-ram --guest-ram-pin --bar1-crossing --atomics-probe --pce-mask-probe --gpga-reserve-probe --map-stress --cross-client-leak --rpc-mixed-allocs'
b='native_raw_rc=0\nbash scripts/fastguest/bare_metal_suite.sh capc_native595 180 '+args+' >/root/prov/sweep_bare595.log 2>&1 || native_raw_rc=$?\necho "NATIVE_RAW_FINDING rc=$native_raw_rc verdict_is_not_a_gate"'
assert a in s;s=s.replace(a,b)
s=s.replace("grep -q '^BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30$' /root/prov/sweep_bare595.log", "grep '^BARE_SUITE_PASS=' /root/prov/sweep_bare595.log || echo NATIVE_RAW_SUMMARY_UNPARSED")
s=s.replace('bash scripts/bench/cuda_ladder.sh host capc_native595 1 >/root/prov/sweep_cuda_host.log 2>&1','native_cuda_rc=0\nbash scripts/bench/cuda_ladder.sh host capc_native595 1 >/root/prov/sweep_cuda_host.log 2>&1 || native_cuda_rc=$?\necho "NATIVE_CUDA_FINDING rc=$native_cuda_rc verdict_is_not_a_gate"')
s=s.replace("assert len(rows)==4 and all('verdict=PASS ' in l and ' rc=0 ' in l for l in rows)","import json\nPath('/root/prov/native_cuda_findings.json').write_text(json.dumps({'rows':rows,'gate':False,'experiment_mode':'OBSERVATION','attestation':False})+'\\n')")
s=s.replace('env KF3_TRANSLATED_CE_RELAY=1 timeout 180 /workspace/native-fixture-target/release/kf-gr-tier >/root/prov/sweep_native_grtier.log 2>&1','native_gr_rc=0\nenv KF3_TRANSLATED_CE_RELAY=1 timeout 180 /workspace/native-fixture-target/release/kf-gr-tier >/root/prov/sweep_native_grtier.log 2>&1 || native_gr_rc=$?\necho "NATIVE_GR_FINDING rc=$native_gr_rc verdict_is_not_a_gate"')
s=s.replace("grep '^GR_TIER_VERDICT=PASS$' /root/prov/sweep_native_grtier.log", "grep '^GR_TIER_VERDICT=' /root/prov/sweep_native_grtier.log || true")
s=s.replace("assert len(allowed)==5 and tuple(G.N.xid_lines(G.N.kernel_snapshot()))==tuple(allowed)","assert tuple(G.N.xid_lines(G.N.kernel_snapshot()))==tuple(allowed)")
needle='bash /root/sweep2/start_secondary.sh';assert s.count(needle)==1
insert='''# B1b: exact 3 pinned repeats + one same-width unpinned control; verdicts are findings.
if bash /root/sweep2/build_fuzz_fixture.sh; then
 flock -o /tmp/kayfabe-fastguest.lock python3 /root/sweep2/fuzz_findings.py \\
  /workspace/bw3-fuzz-target/release/kayfabe-rm-ladder /root/prov/native_fuzz_findings.json
else
 echo NATIVE_FUZZ_FIXTURE_UNRUN_BUILD_FAILURE
 python3 - <<'FUZZ_UNRUN'
from pathlib import Path
import json
Path('/root/prov/native_fuzz_findings.json').write_text(json.dumps({'records':[], 'pinned_samples':0,'native_finding':None,'continue_windows':True,'reason':'fixture build failed','experiment_mode':'OBSERVATION','attestation':False})+'\\n')
FUZZ_UNRUN
fi
[ ! -e /root/prov/XID_STOP ]
touch /root/prov/native_stage_complete
'''
s=s.replace(needle,insert+needle);p.write_text(s)
p=K/'start_secondary.sh';s=p.read_text().replace(" and len(a)==5","");p.write_text(s)
p=D/'ack_native_template.py';s=p.read_text().replace('assert len(allowed)==5 and tuple(G.N.xid_lines(G.N.kernel_snapshot()))==tuple(allowed)','assert tuple(G.N.xid_lines(G.N.kernel_snapshot()))==tuple(allowed)')
s=s.replace("assert 'BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30' in (r/'sweep_bare595.log').read_text()", "raw_summary=[l for l in (r/'sweep_bare595.log').read_text().splitlines() if l.startswith('BARE_SUITE_PASS=')]\nfuzz=json.loads((r/'native_fuzz_findings.json').read_text())")
s=s.replace("assert 'GR_TIER_VERDICT=PASS' in (r/'sweep_native_grtier.log').read_text()", "gr_summary=[l for l in (r/'sweep_native_grtier.log').read_text().splitlines() if l.startswith('GR_TIER_VERDICT=')]")
s=s.replace("assert len(rows)==4 and all('verdict=PASS ' in x for x in rows)","# All native benchmark verdicts are findings in B1b; fault history remains exact.")
s=s.replace("'raw':30,'cuda':4,'gr':15,'native_faults':5,'exact_ordered_fault_match':True,", "'raw_summary':raw_summary,'cuda_rows':rows,'gr_summary':gr_summary,'fuzz':fuzz,\n 'native_faults':len(allowed),'exact_ordered_fault_match':True,\n 'native_verdicts_are_a_gate':False,'experiment_mode':'OBSERVATION','attestation':False,")
p.write_text(s)
p=D/'milestone_probe.py';s=p.read_text();a="out['native_ready'] = (\n    (r / 'sweep_native_grtier.log').exists()\n    and 'GR_TIER_VERDICT=PASS' in (r / 'sweep_native_grtier.log').read_text()\n)";assert a in s;s=s.replace(a,"out['native_ready'] = (r / 'native_stage_complete').exists()")
s=s.replace("'arm_on_observe_b.log',", "'native_fuzz_build.log','native_fuzz_repeat_0.log','native_fuzz_repeat_1.log','native_fuzz_repeat_2.log','native_fuzz_repeat_3.log', 'arm_on_observe_b.log',")
p.write_text(s)
p=D/'export_meta.py';s=p.read_text();s=s.replace("'precomp_tier3.txt'", "'native_fuzz_findings.json','native_cuda_findings.json','native_fuzz_fixture.json','native_fuzz_patch.json','native_fuzz_build.log','native_fuzz_repeat_0.log','native_fuzz_repeat_1.log','native_fuzz_repeat_2.log','native_fuzz_repeat_3.log','precomp_tier3.txt'")
s=s.replace('assert len(b)<=262144','assert len(b)<=1048576').replace('total<=2097152','total<=8388608').replace('len(r.stdout)<=4194304','len(r.stdout)<=12582912');p.write_text(s)
p=D/'final_remote_audit.py';s=p.read_text().replace('assert len(native)==5','assert len(native)<=5').replace('allowed[:5]==native','allowed[:len(native)]==native').replace('allowed[5:]==recorded','allowed[len(native):]==recorded').replace("'native_xid_count':5", "'native_xid_count':len(native)");p.write_text(s)
auth=json.loads(Path('/workspace/bw3-round-b1-run/owner_authority.json').read_text());auth.update(controller='18:15 B1b addendum',stop_usd=2.0,cap_usd=2.5,native_verdicts_are_a_gate=False,pinned_repeats=3,unpinned_controls=1,native_falsifier='ALIAS_REVOKED in>=2of3 valid pinned runs is native finding; root cause not claimed; continue Windows if safety healthy')
(D/'owner_authority.json').write_text(json.dumps(auth,indent=2)+'\n');(T/'owner_authority.json').write_text(json.dumps(auth,indent=2)+'\n')
print('wired native findings and budget')
