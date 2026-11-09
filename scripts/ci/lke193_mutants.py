#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free #305 mutations. Each needs two distinct named failing tests.

No GPU, guest, debugger or network. PowerShell reads only synthetic local fixtures.
Every mutant has its own copied tree; no ratchet is re-recorded in the repository.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT=Path(__file__).resolve().parents[2]
PY='scripts/bench/box/lke193.py'
PS='scripts/bench/windows/lke_decode.ps1'
GATE='scripts/bench/box/windows_gate.sh'
STATE='scripts/bench/windows/gate_state.ps1'
RR='scripts/ci/vast_run_report.py'
CASES=[
 ('equal-count-regresses',PY,'count > maximum','count >= maximum',1),
 ('excess-count-accepted',PY,"(excess if count > maximum else 'PASS')","'PASS'",1),
 ('unknown-borrows-ad102',PY,'row = table.get(die)',"row = table.get(die, table['AD102'])",1),
 ('count-validation-removed',PY,"if type(count) is not int or not 0 <= count <= 0x7fffffff:\n        raise ValueError('missing or invalid live dump count')",'if False:\n        pass',1),
 ('maximum-validation-removed',PY,"if type(count) is not int or not 0 <= count <= 0x7fffffff:\n            raise ValueError('invalid max_count')",'if False:\n            pass',1),
 ('evidence-validation-removed',PY,'if not (isinstance(evidence, dict)', 'if False and (isinstance(evidence, dict)',1),
 ('native-identity-ambiguity-accepted',PY,"if len(lines) != 1 or probe.splitlines().count('PASS native read-only timer map/read/drop x3') != 1:",'if not lines:',1),
 ('malformed-native-identity-ignored',PY,"if sum(line.startswith('host_driver=') for line in probe.splitlines()) != len(lines):",'if False:',1),
 ('partially-parsed-die-table-accepted',PY,"if not entries or re.sub(entry,'',block[1]).replace(',','').strip():",'if False:',1),
 ('unknown-tuple-guessed',PY,"return names.get((arch,implementation),'unknown')","return names.get((arch,implementation),'AD102')",1),
 ('signature-unchecked',PS,"if ([Text.Encoding]::ASCII.GetString($Bytes, 0, 8) -cne 'PAGEDU64')",'if ($false)',1),
 ('parser-length-unchecked',PS,'if ($Bytes.Length -ne 0x1000)', 'if ($false)',1),
 ('short-file-unchecked',PS,'if ($read -ne 0x1000)', 'if ($false)',1),
 ('whole-dump-read',PS,'$bytes = New-Object byte[] 0x1000', '$bytes = [IO.File]::ReadAllBytes($Path)',1),
 ('wrong-bugcheck-offset',PS,'ToUInt32($Bytes, 0x38)','ToUInt32($Bytes, 0x34)',1),
 ('decimal-bugcheck-label',PS,"'0x{0:x8}'","'{0}'",1),
 ('parameter1-wrong-offset',PS,'ToUInt64($Bytes, 0x40)','ToUInt64($Bytes, 0x48)',1),
 ('parameter2-wrong-offset',PS,'ToUInt64($Bytes, 0x48)','ToUInt64($Bytes, 0x40)',1),
 ('parameter3-wrong-offset',PS,'ToUInt64($Bytes, 0x50)','ToUInt64($Bytes, 0x48)',1),
 ('parameter4-wrong-offset',PS,'ToUInt64($Bytes, 0x58)','ToUInt64($Bytes, 0x50)',1),
 ('parameters-truncated',PS,'[BitConverter]::ToUInt64','[BitConverter]::ToUInt32',4),
 ('wer-index-guessed',PS,"$valueKey = 'Sig[' + $Matches[1] + '].Value'","$valueKey = 'Sig[' + ($number - 1) + '].Value'",1),
 ('wer-values-guessed-hex',PS,'$row[$name] = $values[$valueKey]',"$row[$name] = '0x' + $values[$valueKey]",1),
 ('wer-time-omitted',PS,"if ($values.ContainsKey('EventTime'))",'if ($false)',1),
 ('wer-bucket-omitted',PS,"@('Response.BucketId', 'BucketId', 'Response.LegacyBucketId')",'@()',1),
 ('nested-reports-hidden',STATE,"-Filter 'Report.wer' -File -Recurse","-Filter 'Report.wer' -File",2),
 ('bad-header-guessed',PS,"GATE_LKE_HEADER status=REFUSED reason=","GATE_LKE_HEADER bugcheck=0x00000193 status=OK reason=",1),
 ('ratchet-failure-does-not-stop-gate',GATE,'[ "$rc" != 1 ] || fail_guest "LiveKernelEvent 193', '[ "$rc" != 1 ] || true "LiveKernelEvent 193',1),
 ('default-ratchet-gates-again',GATE,'${KF_WINGATE_LKE_RATCHET:-report}','${KF_WINGATE_LKE_RATCHET:-gate}',1),
 ('warn-line-dropped',GATE,'while IFS= read -r line; do say "$line"; done < "$result"',
  'while IFS= read -r line; do [[ "$line" == *"verdict=WARN" ]] || say "$line"; done < "$result"',1),
 ('report-mode-regression-gates',PY,"excess = 'FAIL' if mode == 'gate' else 'WARN'","excess = 'FAIL'",1),
 ('explicit-gate-mode-ignored',PY,"excess = 'FAIL' if mode == 'gate' else 'WARN'","excess = 'WARN'",1),
 ('gate-mode-not-forwarded',GATE,'--mode "${KF_WINGATE_LKE_RATCHET:-report}"','--mode report',1),
 ('strict-mode-ignored',GATE,'|| strict="-Strict"','|| strict=""',1),
 ('non193-reset-check-ignored',GATE,'grep -aq \'^GATE_TDR .* total=0$\' "$EV/tdr_counts.txt" || fail_guest',
  'true || fail_guest',1),
 ('post-idle-probe-check-ignored',GATE,'[ -z "$PROBE_BAD" ] || fail_guest "graded probes failed AFTER',
  '[ -z "$PROBE_BAD" ] || true "graded probes failed AFTER',1),
 ('warn-report-fields-dropped',RR,'verdict=(PASS|WARN|FAIL|UNKNOWN)','verdict=(PASS|FAIL|UNKNOWN)',1),
 ('ratchet-fields-dropped',RR,'elif head == "GATE_LKE_RATCHET":','elif head == "NEVER_LKE_RATCHET":',1),
 ('ratchet-report-invents-pass',RR,'self.windows["live_dumps_193_ratchet"] = m[4]','self.windows["live_dumps_193_ratchet"] = "PASS"',1),
 ('entries-counted-as-reports',STATE,'$lke193 = $lke193Reports.Count','$lke193 = $lke193Entries',1),
 ('entry-record-used-as-report-identity',STATE,'$key = $report.key',"$key = 'entry:' + $e.RecordId",1),
 ('own-report-event-time-omitted',STATE,"$identity + '|time:' + $ownTime",'$identity',1),
 ('entry-time-used-for-staleness',STATE,'$rt = $report.time','$rt = $e.TimeCreated.ToUniversalTime()',1),
 ('unidentified-events-merged-by-signature',STATE,'if ($key) { $lke193Reports[$key] = $true }',"$lke193Reports[(Event-Signature $e)] = $true",1),
 ('193-resubmissions-become-other-events',STATE,'$lkeOther = $lke.Count - $lke193Entries','$lkeOther = $lke.Count - $lke193',1),
 ('real-code-label-ignored',PS,"@('Code', 'Parameter 1'","@('IGNORED', 'Parameter 1'",1),
 ('real-parameter-labels-ignored',PS,"'Parameter 1', 'Parameter 2', 'Parameter 3', 'Parameter 4'","'IGNORED1', 'IGNORED2', 'IGNORED3', 'IGNORED4'",1),
 ('only-watchdog-subdirectory-searched',STATE,"@('C:\\Windows\\LiveKernelReports', (Join-Path $wer", "@('C:\\Windows\\LiveKernelReports\\WATCHDOG', (Join-Path $wer",1),
 ('only-watchdog-filenames-searched',STATE,"-Filter '*.dmp'","-Filter 'WATCHDOG-*.dmp'",1),
 ('archive-dump-headers-hidden',STATE,", (Join-Path $wer 'ReportArchive')",'',1),
 ('queue-dump-headers-hidden',STATE,", (Join-Path $wer 'ReportQueue')",'',1),
 ('ratchet-entry-unit-admitted',PY,"if row.get('count_unit') != 'distinct_reports':",'if False:',1),
 ('raw-entries-compared-to-report-limit',PY,'count > maximum','(entries if entries is not None else count) > maximum',1),
 ('raw-count-log-line-dropped',PY,"        print('GATE_LKE_COUNTS reports=","        if False: print('GATE_LKE_COUNTS reports=",1),
 ('raw-count-report-fields-dropped',RR,'elif head == "GATE_LKE_COUNTS":','elif head == "NEVER_LKE_COUNTS":',1),
 ('raw-entry-validation-removed',PY,'if value is not None and (type(value) is not int or not 0 <= value <= 0x7fffffff):','if False:',1),
 ('inconsistent-counts-accepted',PY,'if entries is not None and (entries < count or (unresolved is not None and unresolved > entries)):','if False:',1),
 ('unresolved-comparison-invented',PY,"row is None or unresolved","row is None",1),
 ('explicit-identity-overridden-by-signature',STATE,'$signature -and -not $id -and -not $directory -and (Is-193 $e)','$signature',1),
 ('strict-counts-reports-instead-of-entries',STATE,'if ($Strict) { $total += $lke193Entries }','if ($Strict) { $total += $lke193 }',1),
]
FILES=[PY,PS,GATE,RR,'scripts/ci/test_vast_lke193.py','scripts/ci/vast_report.py','scripts/ci/app_policy.py',
 'scripts/ci/test_vast_lke193_counts.py',
 'scripts/bench/windows/gate_state.ps1','scripts/bench/box/known_live_dumps_193.json',
 'scripts/apps/known_unsupported.json','scripts/bench/windows/appmatrix/gate_profiles.json',
 'crates/kf-chip/src/lib.rs','crates/kf-chip/src/die.rs',
 'traces/windows_gate_20261101/g4_windows.log','traces/turing_boot_20261103/README.md','traces/ampere_hw_20261103/README.md']
FILES += ['traces/turing_boot_20261103/boxB/tbB1/wer_reports.txt','traces/ampere_hw_20261103/boxA/am2_win/wer_reports.txt']
FILES += [str(path.relative_to(ROOT)) for path in (ROOT/'traces/lke193_phase0_20261109').rglob('*') if path.is_file()]
COLLECT='''import io,json,sys,unittest
s=unittest.defaultTestLoader.discover('.',pattern='test_vast_lke193*.py')
def cases(suite):
 for test in suite:
  if isinstance(test,unittest.TestSuite): yield from cases(test)
  else: yield test
if sys.argv[1]=='fast':
 s=unittest.TestSuite(t for t in cases(s) if t.__class__.__name__ not in ('GuestDecode','DistinctReports'))
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'skips':len(r.skipped),'killers':sorted(set(getattr(t,'test_case',t).id() for t,_ in r.failures+r.errors))}))
'''

def run(root, powershell=True):
 p=subprocess.run([sys.executable,'-c',COLLECT,'all' if powershell else 'fast'],cwd=root/'scripts/ci',capture_output=True,text=True,timeout=180,
                  env={'PATH':os.environ['PATH'],'PYTHONDONTWRITEBYTECODE':'1'})
 if p.returncode: raise RuntimeError('harness failed: '+p.stderr[-500:])
 return json.loads(p.stdout.splitlines()[-1])


def main():
 with tempfile.TemporaryDirectory(prefix='dx1-lke193-') as tmp:
  root=Path(tmp)
  for name in FILES:
   path=root/name; path.parent.mkdir(parents=True,exist_ok=True); shutil.copyfile(ROOT/name,path)
  baseline=run(root)
  if baseline['killers'] or baseline['skips']:
   print('BASELINE NOT GREEN: '+str(baseline)); return 1
  rows=[]
  for name,file,old,new,count in CASES:
   path=root/file; source=path.read_text()
   if source.count(old)!=count: raise RuntimeError(name+': anchor count '+str(source.count(old)))
   # Python/report/shell mutations cannot change the PowerShell fixture modules.
   # Always run the complete baseline and the complete decoder/counter mutations.
   path.write_text(source.replace(old,new)); result=run(root,powershell=file in (PS,STATE)); path.write_text(source)
   row=dict(mutant=name,tests=result['tests'],count=len(result['killers']),killers=result['killers']); rows.append(row)
   print(name+': '+str(row['count'])+' killers',file=sys.stderr,flush=True)
  print(json.dumps(dict(tests=baseline['tests'],cases=rows),indent=2))
  return int(any(row['count']<2 for row in rows))


if __name__=='__main__': sys.exit(main())
