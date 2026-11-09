"""Phase-0 counter controls: event entries are not distinct WER reports.

Only local synthetic files and mocked event APIs run. The h1/h2 replay input is
coordinator evidence; signatures resolve existing report identities, never replace them.
"""
import datetime as dt
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]
PWSH=shutil.which('pwsh') or ('/root/am/pwsh/pwsh' if Path('/root/am/pwsh/pwsh').exists() else None)
RID='fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001'


def filetime(iso):
    value=dt.datetime.fromisoformat(iso.replace('Z','+00:00'))
    return str(int((value-dt.datetime(1601,1,1,tzinfo=dt.timezone.utc)).total_seconds()*10000000))


def report(directory='fresh', identity=RID, when='2026-11-09T02:20:15Z', code='193', pointer='fffffa4efa4e0001'):
    names=['Code','Parameter 1','Parameter 2','Parameter 3','Parameter 4','OS version','Service Pack','Product']
    values=[code,'815','2',pointer,'0','10_0_26100','0_0','256_1']
    text=['EventType=LiveKernelEvent','EventTime='+filetime(when)]
    if identity: text.append('ReportIdentifier='+identity)
    for i,(name,value) in enumerate(zip(names,values)):
        text.extend([f'Sig[{i}].Name={name}',f'Sig[{i}].Value={value}'])
    return {'directory':directory,'text':'\n'.join(text)}


def event(index=1, identity=RID, directory=None, code='193', pointer='fffffa4efa4e0001'):
    message=f'Event Name: LiveKernelEvent P1: {code} P2: 815 P3: 2 P4: {pointer} P5: 0 P6: 10_0_26100 P7: 0_0 P8: 256_1'
    if identity: message+=' Report Id: '+identity
    if directory: message+=' Files: C:\\ProgramData\\Microsoft\\Windows\\WER\\ReportQueue\\'+directory+'\\Report.wer'
    return {'message':message,'when':f'2026-11-09T02:21:{index:02d}Z','record':index}


@unittest.skipUnless(PWSH,'PowerShell required for the CPU-only report counter controls')
class DistinctReports(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp=tempfile.TemporaryDirectory(); root=Path(cls.tmp.name)
        cases={}
        def add(name,reports,events,strict=False,system_count=0):
            cases[name]={'reports':reports,'events':events,'strict':strict,'system_count':system_count}
        repeated=[event(i) for i in (1,2,3)]
        add('id',[report()],repeated)
        add('path',[report(identity=None)],[event(i,identity=None,directory='fresh') for i in (1,2,3)])
        add('copies',[report('ReportQueue/fresh'),report('ReportArchive/fresh')],repeated)
        later=report('later',when='2026-11-09T02:20:16Z')
        add('reused_id',[report(),later],[event(1,directory='fresh'),event(2,directory='later')])
        ticks=report('ticks'); ticks['text']=ticks['text'].replace(filetime('2026-11-09T02:20:15Z'),str(int(filetime('2026-11-09T02:20:15Z'))+1))
        add('subsecond',[report(),ticks],[event(1,directory='fresh'),event(2,directory='ticks')])
        other_id=RID[:-1]+'2'
        add('same_signature',[report(),report('second',identity=other_id)],
            [event(1),event(2,identity=other_id)])
        old=report(when='2026-11-08T02:20:15Z')
        add('stale',[old],repeated)
        add('stale_other',[report(when='2026-11-08T02:20:15Z',code='141')],[event(1,code='141')])
        add('pruned',[],repeated)
        add('missing_id',[old],[event(1,identity=other_id)])
        add('missing_path',[old],[event(1,identity=None,directory='missing')])
        add('unidentified_other',[report(when='2026-11-08T02:20:15Z',code='141')],[event(1,identity=None,code='141')])
        add('unidentified',[],[event(i,identity=None) for i in (1,2,3)])
        add('unidentified_mixed',[],[event(1,identity=None),event(2,identity=None),
            event(3,identity=None,pointer='fffffa4efa4e0002'),event(4,identity=None,pointer='fffffa4efa4e0002')])
        add('mixed',[report()],repeated+[event(4,identity=None,code='141'),event(5,identity=None,code='117')])
        add('strict',[report()],repeated,strict=True)
        add('strict_unidentified',[],[event(i,identity=None) for i in (1,2,3)],strict=True)
        add('reset',[report()],repeated,system_count=1)
        add('empty',[],[])
        # Replay the recorded WER spellings and P1..P8 event signatures without
        # inventing event ReportIds that the coordinator's 900-char view omitted.
        for boot in ('p193_h1','p193_h2'):
            folder=ROOT/'traces/lke193_phase0_20261109'/boot
            records={}
            for line in (folder/'g_wer.txt').read_text().splitlines():
                if line.startswith('GATE_LKE report='):
                    name=re.search(r'report="([^"]+)"',line)[1]
                    records[name]=['EventType=LiveKernelEvent','EventTime='+re.search(r'event_time_raw="(\d+)"',line)[1]]
                elif line.startswith('GATE_WER_PARAM '):
                    _,name,value=line.split(' ',2)
                    if name in records: records[name].append(value)
            entries=[]
            for i,line in enumerate((folder/'events_193.txt').read_text().splitlines(),1):
                entries.append({'message':line.split(' msg=',1)[1],
                                'when':line.split()[2],'record':i})
            add(boot,[{'directory':name,'text':'\n'.join(lines)} for name,lines in records.items()],entries)
        state=(ROOT/'scripts/bench/windows/gate_state.ps1').read_text()
        manifest=[]
        for name,case in cases.items():
            work=root/name; work.mkdir()
            (work/'dumps').mkdir()
            (work/'wer').mkdir()
            for record in case['reports']:
                relative=record['directory']
                if not relative.startswith(('ReportQueue/','ReportArchive/')): relative='ReportQueue/'+relative
                directory=work/'wer'/relative; directory.mkdir(parents=True)
                (directory/'Report.wer').write_text(record['text'],encoding='utf-16')
            modified=state.replace("'C:\\ProgramData\\Microsoft\\Windows\\WER'",repr(str(work/'wer')))
            modified=modified.replace("'C:\\Windows\\LiveKernelReports\\WATCHDOG'",repr(str(work/'dumps/WATCHDOG')))
            modified=modified.replace("'C:\\Windows\\LiveKernelReports'",repr(str(work/'dumps')))
            modified=modified.replace('C:\\Windows\\LiveKernelReports',repr(str(work/'dumps')))
            (work/'gate_state.ps1').write_text(modified)
            shutil.copyfile(ROOT/'scripts/bench/windows/lke_decode.ps1',work/'lke_decode.ps1')
            manifest.append(dict(name=name,path=str(work/'gate_state.ps1'),**case))
        (root/'cases.json').write_text(json.dumps(manifest))
        script=root/'run.ps1'
        script.write_text('''param($Cases)
function Get-WinEvent {
 param($FilterHashtable,$ErrorAction)
 if ($FilterHashtable.LogName -eq 'Application') {
  foreach ($event in $global:Current.events) {
   [pscustomobject]@{Message=$event.message;TimeCreated=[DateTime]::Parse($event.when).ToUniversalTime();RecordId=$event.record}
  }
 } elseif ($FilterHashtable.ProviderName -eq 'nvlddmkm') {
  for ($i=0;$i -lt $global:Current.system_count;$i++) { [pscustomobject]@{Message='synthetic reset'} }
 }
}
$out=@{}
foreach ($case in (Get-Content -LiteralPath $Cases -Raw | ConvertFrom-Json)) {
 $global:Current=$case
 if ($case.strict) { $out[$case.name]=@(& $case.path -Mode tdr -Since '2026-11-09T02:00:00Z' -Strict) }
 else { $out[$case.name]=@(& $case.path -Mode tdr -Since '2026-11-09T02:00:00Z') }
}
$out | ConvertTo-Json -Depth 8 -Compress
''')
        done=subprocess.run([PWSH,'-NoProfile','-File',str(script),str(root/'cases.json')],capture_output=True,text=True,timeout=90)
        if done.returncode: raise AssertionError(done.stderr+done.stdout)
        cls.data=json.loads(done.stdout)

    @classmethod
    def tearDownClass(cls): cls.tmp.cleanup()

    def counts(self,name):
        line=next(line for line in self.data[name] if line.startswith('GATE_TDR '))
        return dict(re.findall(r'(\w+)=(\S+)',line))
    def check(self,name,reports,entries,stale=0,other=0,total=0):
        row=self.counts(name)
        self.assertEqual(tuple(int(row[k]) for k in ('livekernelevent193','livekernelevent193_entries',
            'livekernelevent_stale','livekernelevent_other','total')),(reports,entries,stale,other,total))
    def test_repeated_report_id_is_one_report(self): self.check('id',1,3)
    def test_repeated_report_path_is_one_report(self): self.check('path',1,3)
    def test_archive_queue_copies_do_not_double_count(self): self.check('copies',1,3)
    def test_report_id_plus_event_time_keeps_distinct_reports(self): self.check('reused_id',2,2)
    def test_subsecond_report_event_times_are_not_truncated(self): self.check('subsecond',2,2)
    def test_equal_signatures_do_not_merge_different_report_ids(self): self.check('same_signature',2,2)
    def test_phase0_h1_three_entries_are_one_fresh_report(self): self.check('p193_h1',1,3,stale=2)
    def test_phase0_h2_three_entries_are_one_fresh_report(self): self.check('p193_h2',1,3,stale=2)
    def test_stale_report_time_still_overrides_new_entry_times(self): self.check('stale',0,0,stale=3)
    def test_stale_other_code_entries_are_still_excluded(self): self.check('stale_other',0,0,stale=1)
    def test_pruned_report_id_still_deduplicates_without_guessing_time(self):
        self.check('pruned',1,3)
        self.assertEqual(self.counts('pruned')['livekernelevent193_unresolved'],'3')
    def test_missing_explicit_id_never_borrows_a_stale_report_signature(self):
        self.check('missing_id',1,1)
        self.assertEqual(self.counts('missing_id')['livekernelevent193_unresolved'],'1')
    def test_missing_explicit_path_never_borrows_a_stale_report_signature(self):
        self.check('missing_path',1,1)
        self.assertEqual(self.counts('missing_path')['livekernelevent193_unresolved'],'1')
    def test_unidentified_non193_entry_still_fails_closed(self):
        self.check('unidentified_other',0,0,other=1,total=1)
    def test_unidentified_entries_are_not_merged_by_signature(self):
        self.check('unidentified',0,3)
        self.assertEqual(self.counts('unidentified')['livekernelevent193_unresolved'],'3')
    def test_unidentified_signature_groups_are_not_fabricated_reports(self):
        self.check('unidentified_mixed',0,4)
        self.assertEqual(self.counts('unidentified_mixed')['livekernelevent193_unresolved'],'4')
    def test_non193_counts_do_not_include_193_resubmissions(self): self.check('mixed',1,3,other=2,total=2)
    def test_strict_mode_still_counts_any_193_entry(self): self.check('strict',1,3,total=3)
    def test_strict_mode_gates_unresolved_entries_with_zero_report_count(self):
        self.check('strict_unidentified',0,3,total=3)
        self.assertEqual(self.counts('strict_unidentified')['livekernelevent193_unresolved'],'3')
    def test_other_reset_provider_still_gates(self): self.check('reset',1,3,total=1)
    def test_no_entries_are_explicit_zero_counts(self): self.check('empty',0,0)
