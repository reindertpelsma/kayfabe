from pathlib import Path
import json,subprocess,sys
root=Path(__file__).resolve().parents[3]; guard=root/'scripts/bench/native_xid_guard.py'; suite=root/'scripts/bench/guarded_fast_suite.py'
original=guard.read_text(); original_suite=suite.read_text()
mutants=[
('ignore_actual_qemu_pid',guard,"int(record['pid']) != context['pid'] or",'False or'),
('ignore_host_guest_channel_pair',guard,'if i < len(twins) and twins[i] !=','if False and twins[i] !='),
('ignore_fault_address',guard,"int(record['va'].replace('_', ''), 16) != va or",'False or'),
('allow_non31_code',guard,"int(record['code']) != 31 or",'False or'),
('ignore_fault_access_type',guard,"record['fault'] != fault or record['access'] != access",'False'),
('skip_completed_recovery_proof',guard,"operations = _guest_operations(serial, context['arm'], final)","operations = _guest_operations(serial, context['arm'], False)"),
('allow_native_history_replacement',guard,'if len(baseline) < 5 or lines[:len(baseline)] != tuple(baseline):','if len(baseline) < 5:'),
('allow_trailing_payload',guard,'match = re.fullmatch(','match = re.search('),
('allow_hidden_first_xid',guard,"if line.count('NVRM: Xid') != 1:",'if False:'),
('skip_context_provenance',guard,'    _context(context)\n',''),
('ignore_local_source_changes',guard,'if local != committed or len(local) > MAX_BYTES:','if len(local) > MAX_BYTES:'),
('skip_live_identity_recheck',guard,"    identity = process_identity(context['pid'])\n    if (identity is None or identity[1] != context['starttime'] or\n            str(Path('/proc', str(context['pid']), 'exe').resolve()) != context['exe']):\n        raise GuardRefused('owned QEMU starttime/executable changed or disappeared')",'    return'),
('allow_unowned_process_capture',guard,'if pid <= 1 or identity is None or identity[0] != os.getpid():','if identity is None:'),
('ignore_route',guard,"record['engine'] != 'CE0' or record['client'] != 'HUBCLIENT_CE1' or",'False or'),
('ignore_pci',guard,"record['pci'] != pci[1] or",'False or'),
('allow_reduced_native_baseline',guard,"    if len(expected) != 5:\n        raise GuardRefused('approved native baseline requires exactly five source faults')\n",''),
('ignore_final_twin_count',guard,'(final and len(twins) != count)','False'),
('ignore_capture_executable',guard,"    if executable != str(Path(binary).resolve(strict=True)):\n        raise GuardRefused('QEMU executable identity differs')\n",''),
('skip_binary_revision',guard,"    if (not re.fullmatch(r'[0-9a-f]{40}', revision) or\n            Path(binary).parent.name != revision[:8] or Path(binary).name != 'qemu-system-x86_64'):\n        raise GuardRefused('binary is not in its pinned revision directory')",'    return'),
('skip_one_arm_primitive',guard,"if len(tokens) != 2 or tokens[1] != '--probe-launch-dma' or not re.fullmatch(r'--[a-z0-9-]+', tokens[0]):",'if False:'),
('leave_owned_child_running',guard,"        if child.poll() is None:\n            child.terminate()  # Popen-owned child only; never a process pattern.\n            child.wait(timeout=30)",'        pass'),
('continue_after_failed_arm',suite,"            return 7\n",'            continue\n'),
]
runner="""import unittest,json,sys
suite=unittest.defaultTestLoader.discover('scripts/bench/tests',pattern='test_*guard*.py')
r=unittest.TextTestRunner(stream=sys.stderr,verbosity=0).run(suite)
print('MUTATION_RESULT='+json.dumps({'tests':r.testsRun,'failures':[t.id() for t,x in r.failures],'errors':[t.id() for t,x in r.errors]}))
"""
results=[]; out=root/'traces/sweep_ada_20261018/exit_race/matching_mutants'; out.mkdir(exist_ok=True)
try:
 for name,path,old,new in mutants:
  text=original if path==guard else original_suite
  assert text.count(old)==1,(name,text.count(old))
  guard.write_text(original);suite.write_text(original_suite)
  path.write_text(text.replace(old,new,1))
  r=subprocess.run([sys.executable,'-B','-c',runner],cwd=root,capture_output=True,text=True,timeout=30)
  line=[l for l in r.stdout.splitlines() if l.startswith('MUTATION_RESULT=')]
  assert len(line)==1,(name,r.stderr[-1000:])
  record=json.loads(line[0].split('=',1)[1]);record['mutant']=name
  assert not record['errors'],(name,record)
  assert len(record['failures'])>=2,(name,record)
  results.append(record)
  (out/(name+'.json')).write_text(json.dumps(record,indent=2)+'\n')
  print(name,len(record['failures']),flush=True)
finally:
 guard.write_text(original);suite.write_text(original_suite)
(out/'kill_table.json').write_text(json.dumps(results,indent=2)+'\n')
