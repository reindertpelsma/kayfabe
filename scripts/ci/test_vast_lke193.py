"""GPU-free issue #305 tests. Falsifier: a default excess gates or loses WARN, an unknown die
borrows a baseline, WER fields are inferred as hex, or an unvalidated dump is decoded.
Synthetic headers use the owner's public PAGE/DU64 offsets, not a captured dump.
"""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / 'scripts/bench/box/lke193.py'
PS = ROOT / 'scripts/bench/windows/lke_decode.ps1'
spec = importlib.util.spec_from_file_location('lke193', HELPER)
L = importlib.util.module_from_spec(spec)
spec.loader.exec_module(L)
PWSH = shutil.which('pwsh') or ('/root/am/pwsh/pwsh' if Path('/root/am/pwsh/pwsh').exists() else None)

class Ratchet(unittest.TestCase):
    def test_seeds_match_the_owner_measurements(self):
        table = L.load_ratchet()
        self.assertEqual({k:v['max_count'] for k,v in table.items()}, {'AD102':1,'TU106':1,'GA102':1})
        self.assertEqual({k:v['legacy_entry_max_count'] for k,v in table.items()}, {'AD102':2,'TU106':3,'GA102':4})
        for row in table.values():
            self.assertEqual(row['count_unit'],'distinct_reports')
            self.assertTrue((ROOT / row['source_evidence']['path']).is_file())
            self.assertRegex(row['source_evidence']['revision'], r'^[0-9a-f]{7,40}$')

    def test_ad102_at_the_max_is_non_gating(self):
        self.assertEqual(L.decide('AD102',1)['verdict'],'PASS')
    def test_ga102_at_the_max_is_non_gating(self):
        self.assertEqual(L.decide('GA102',1)['verdict'],'PASS')
    def test_entry_unit_cannot_be_used_as_a_report_limit(self):
        table=L.load_ratchet(); table['AD102']['count_unit']='event_entries'
        with self.assertRaises(ValueError): L.validate_table(table)
    def test_unlabelled_limit_cannot_be_silently_interpreted(self):
        table=L.load_ratchet(); del table['TU106']['count_unit']
        with self.assertRaises(ValueError): L.validate_table(table)
    def test_ad102_excess_warns_by_default(self):
        self.assertEqual(L.decide('AD102',3)['verdict'],'WARN')
    def test_tu106_excess_warns_by_default(self):
        self.assertEqual(L.decide('TU106',4)['verdict'],'WARN')
    def test_ad102_explicit_gate_mode_still_fails(self):
        self.assertEqual(L.decide('AD102',3,mode='gate')['verdict'],'FAIL')
    def test_turing_explicit_gate_mode_still_fails(self):
        self.assertEqual(L.decide('TU106',4,mode='gate')['verdict'],'FAIL')
    def test_no_neighbours_baseline_for_ad104(self):
        self.assertEqual(L.decide('AD104',99)['verdict'],'UNKNOWN')
    def test_no_neighbours_baseline_for_unknown(self):
        self.assertEqual(L.decide('unknown',99)['verdict'],'UNKNOWN')
    def test_missing_count_on_a_seeded_die_cannot_pass(self):
        with self.assertRaises(ValueError): L.decide('AD102',None)
    def test_negative_count_cannot_pass(self):
        with self.assertRaises(ValueError): L.decide('TU106',-1)
    def test_bool_count_cannot_pass(self):
        with self.assertRaises(ValueError): L.decide('GA102',True)
    def test_seeded_limits_cannot_be_negative(self):
        table=L.load_ratchet(); table['AD102']['max_count']=-1
        with self.assertRaises(ValueError): L.validate_table(table)
    def test_seeded_limits_cannot_be_booleans(self):
        table=L.load_ratchet(); table['AD102']['max_count']=True
        with self.assertRaises(ValueError): L.validate_table(table)
    def test_seeded_evidence_needs_a_revision(self):
        table=L.load_ratchet(); del table['AD102']['source_evidence']['revision']
        with self.assertRaises(ValueError): L.validate_table(table)
    def test_seeded_limits_need_evidence(self):
        with self.assertRaises(ValueError):
            L.validate_table({'AD102':dict(max_count=2,count_unit='distinct_reports',source_evidence={})})
    def test_large_regression_cannot_escape_the_ratchet(self):
        self.assertEqual(L.decide('GA102',100001,mode='gate')['verdict'],'FAIL')
    def test_large_turing_regression_cannot_escape_the_ratchet(self):
        self.assertEqual(L.decide('TU106',100001,mode='gate')['verdict'],'FAIL')
    def test_check_never_updates_the_ratchet(self):
        p=HELPER.with_name('known_live_dumps_193.json'); before=p.read_bytes()
        L.decide('AD102',999)
        self.assertEqual(p.read_bytes(),before)
    def test_raw_entries_are_preserved_separately_from_reports(self):
        row=L.decide('AD102',1,entries=3)
        self.assertEqual((row['count'],row['entries'],row['unit'],row['verdict']),(1,3,'distinct_reports','PASS'))
    def test_gate_mode_does_not_compare_raw_entries_with_report_maximum(self):
        row=L.decide('TU106',1,mode='gate',entries=3)
        self.assertEqual((row['count'],row['entries'],row['verdict']),(1,3,'PASS'))
    def test_raw_entries_cannot_be_negative(self):
        with self.assertRaises(ValueError): L.decide('AD102',1,entries=-1)
    def test_raw_entries_cannot_be_booleans(self):
        with self.assertRaises(ValueError): L.decide('GA102',1,entries=True)
    def test_unresolved_entries_cannot_be_negative(self):
        with self.assertRaises(ValueError): L.decide('AD102',1,entries=3,unresolved=-1)
    def test_report_count_cannot_exceed_entry_count(self):
        with self.assertRaises(ValueError): L.decide('AD102',2,entries=1)
    def test_unresolved_entries_cannot_exceed_entry_count(self):
        with self.assertRaises(ValueError): L.decide('TU106',1,entries=3,unresolved=4)
    def test_unresolved_identity_cannot_fabricate_a_below_limit_pass(self):
        self.assertEqual(L.decide('AD102',0,entries=3,unresolved=3)['verdict'],'UNKNOWN')
    def test_unresolved_report_time_cannot_fabricate_a_gate_failure(self):
        self.assertEqual(L.decide('TU106',3,mode='gate',entries=4,unresolved=1)['verdict'],'UNKNOWN')

class Identity(unittest.TestCase):
    def probe(self, arch, impl):
        return f'host_driver=580.159.04 arch=({arch}, {impl}, 161)\nPASS native read-only timer map/read/drop x3\n'
    def test_ada_uses_the_derived_table(self): self.assertEqual(L.identify(self.probe(0x190,2),ROOT),'AD102')
    def test_turing_uses_the_derived_table(self): self.assertEqual(L.identify(self.probe(0x160,6),ROOT),'TU106')
    def test_ampere_uses_the_derived_table(self): self.assertEqual(L.identify(self.probe(0x170,2),ROOT),'GA102')
    def test_unknown_tuple_has_no_guessed_die(self): self.assertEqual(L.identify(self.probe(0x999,2),ROOT),'unknown')
    def test_unknown_impl_has_no_guessed_die(self): self.assertEqual(L.identify(self.probe(0x190,99),ROOT),'unknown')
    def test_marketing_name_is_not_identity(self):
        with self.assertRaises(ValueError): L.identify('RTX 4090 AD102\n',ROOT)
    def test_conflicting_native_identities_are_refused(self):
        probe=self.probe(0x190,2)+'host_driver=580.159.04 arch=(368, 2, 161)\n'
        with self.assertRaises(ValueError): L.identify(probe,ROOT)
    def test_duplicate_native_identity_is_refused(self):
        with self.assertRaises(ValueError): L.identify(self.probe(0x190,2)*2,ROOT)
    def test_malformed_second_native_identity_is_refused(self):
        with self.assertRaises(ValueError): L.identify(self.probe(0x190,2)+'host_driver=broken\n',ROOT)
    def test_partial_second_native_identity_is_refused(self):
        with self.assertRaises(ValueError): L.identify(self.probe(0x170,2)+'host_driver=580.159.04 arch=(oops)\n',ROOT)
    def damaged_table(self, source):
        with tempfile.TemporaryDirectory() as tmp:
            dest=Path(tmp,'crates/kf-chip/src'); dest.mkdir(parents=True)
            shutil.copyfile(ROOT/'crates/kf-chip/src/lib.rs',dest/'lib.rs')
            (dest/'die.rs').write_text(source)
            with self.assertRaises(ValueError): L.identify(self.probe(0x190,2),tmp)
    def test_unparsed_derived_table_cannot_silently_be_unknown(self):
        self.damaged_table('pub const DIE_NAMES: &[(u32,u32,&str)] = &[\n    unsupported_format,\n];')
    def test_partially_unparsed_table_cannot_skip_a_seeded_die(self):
        source=(ROOT/'crates/kf-chip/src/die.rs').read_text()
        self.damaged_table(source.replace('(arch::AD100, 0x2, "AD102")','unsupported_format'))

@unittest.skipUnless(PWSH,'PowerShell is required for the CPU-only guest parser tests')
class GuestDecode(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp=tempfile.TemporaryDirectory(); root=Path(cls.tmp.name)
        params=[0x815,0x100000002,0xfedcba9876543210,0xffffffffffffffff]
        blob=bytearray(b'\xa5'*4096); blob[:8]=b'PAGEDU64'
        struct.pack_into('<I',blob,0x38,0x193)
        for offset,value in zip((0x40,0x48,0x50,0x58),params): struct.pack_into('<Q',blob,offset,value)
        real=bytearray(0x1000); real[:8]=b'PAGEDU64'; struct.pack_into('<I',real,0x38,0x193)
        for offset,value in zip((0x40,0x48,0x50,0x58),(0x815,2,0xfffffa4efa4e0001,0)):
            struct.pack_into('<Q',real,offset,value)
        cases={'good':blob, 'real':real+b'\0'*(262144-4096), 'c1':bytearray(blob), 'bad_first':bytearray(blob), 'bad_second':bytearray(blob),
               'short':blob[:4095], 'tiny':blob[:95], 'long':blob+b'poison'*1000}
        struct.pack_into('<I',cases['c1'],0x38,0xc1)
        struct.pack_into('<Q',cases['c1'],0x40,0x1122334455667788)
        cases['bad_first'][0]=0; cases['bad_second'][7]=0
        for name,data in cases.items(): (root/(name+'.dmp')).write_bytes(data)
        wer='\n'.join(['EventType=LiveKernelEvent','EventTime=134123456780000000','Response.BucketId=bucket=watchdog',
             'Sig[7].Name=P1','Sig[7].Value=193','Sig[0].Name=P2','Sig[0].Value=815','Sig[1].Name=P3','Sig[1].Value=2',
             'Sig[2].Name=P4','Sig[2].Value=ffff111122223333','Sig[3].Name=P5','Sig[3].Value=5',
             'Sig[4].Name=P6','Sig[4].Value=6','Sig[5].Name=P7','Sig[5].Value=7','Sig[6].Name=P8','Sig[6].Value=8'])
        (root/'Report.wer').write_text(wer,encoding='utf-16')
        real_wer='\n'.join(['EventType=LiveKernelEvent','EventTime=134386653875328118',
            'Sig[5].Name=Code','Sig[5].Value=193','Sig[3].Name=Parameter 1','Sig[3].Value=815',
            'Sig[0].Name=Parameter 2','Sig[0].Value=2','Sig[7].Name=Parameter 3','Sig[7].Value=fffffa4efa4e0001',
            'Sig[1].Name=Parameter 4','Sig[1].Value=0','Sig[2].Name=OS version','Sig[2].Value=10_0_26100',
            'Sig[4].Name=Service Pack','Sig[4].Value=0_0','Sig[6].Name=Product','Sig[6].Value=256_1'])
        (root/'real.wer').write_text(real_wer,encoding='utf-16')
        archive=root/'wer/ReportArchive'
        for relative in ('direct/Report.wer','nested/deeper/Report.wer','ordinary/Report.wer'):
            report=archive/relative; report.parent.mkdir(parents=True,exist_ok=True)
            report.write_text(wer if 'ordinary' not in relative else 'EventType=AppCrash\nSig[0].Name=P1\nSig[0].Value=ordinary',encoding='utf-16')
        watchdog=root/'dumps/WATCHDOG'; watchdog.mkdir(parents=True)
        (watchdog/'WATCHDOG-synthetic.dmp').write_bytes(blob)
        (root/'dumps/root.dmp').write_bytes(real)
        nested=root/'dumps/OTHER/nested'; nested.mkdir(parents=True)
        (nested/'other-name.dmp').write_bytes(real)
        for store in ('ReportArchive','ReportQueue'):
            directory=root/'wer'/store/'real'; directory.mkdir(parents=True)
            (directory/'Report.wer').write_text(real_wer,encoding='utf-16')
            (directory/'same-name.dmp').write_bytes(real)
        state=(ROOT/'scripts/bench/windows/gate_state.ps1').read_text()
        state=state.replace("'C:\\ProgramData\\Microsoft\\Windows\\WER'", "'"+str(root/'wer')+"'")
        state=state.replace("'C:\\Windows\\LiveKernelReports\\WATCHDOG'", "'"+str(watchdog)+"'")
        state=state.replace("'C:\\Windows\\LiveKernelReports'", "'"+str(root/'dumps')+"'")
        state=state.replace('C:\\Windows\\LiveKernelReports', "'"+str(root/'dumps')+"'")
        (root/'gate_state.ps1').write_text(state)
        shutil.copyfile(PS,root/'lke_decode.ps1')
        script=root/'check.ps1'
        script.write_text('''param($Module,$Root)
. $Module
$out=@{}
foreach ($name in @('good','real','c1','bad_first','bad_second','short','tiny','long')) {
 try { $out[$name]=Read-LkeDumpHeader (Join-Path $Root ($name+'.dmp')) } catch { $out[$name]=@{refused=$_.Exception.Message} }
}
foreach ($size in @(96,4095)) {
 $short=New-Object byte[] $size
 [Array]::Copy([IO.File]::ReadAllBytes((Join-Path $Root 'good.dmp')),0,$short,0,$size)
 try { $out['direct'+$size]=ConvertFrom-LkeDumpHeader $short } catch { $out['direct'+$size]=@{refused=$_.Exception.Message} }
}
$out['bad_second_lines']=@(Write-LkeDumpHeader (Join-Path $Root 'bad_second.dmp'))
$out['wer']=ConvertFrom-LkeWerReport ([IO.File]::ReadAllText((Join-Path $Root 'Report.wer')))
$out['real_wer']=ConvertFrom-LkeWerReport ([IO.File]::ReadAllText((Join-Path $Root 'real.wer')))
$out['real_lines']=@(Write-LkeWerLine ([IO.File]::ReadAllText((Join-Path $Root 'real.wer'))) 'real' 'ReportQueue')
$out['missing']=ConvertFrom-LkeWerReport "EventType=LiveKernelEvent`nEventTime=invalid"
$out['partial']=ConvertFrom-LkeWerReport "EventType=LiveKernelEvent`nSig[3].Name=P1`nSig[3].Value=193"
$out['lines']=@(Write-LkeWerLine ([IO.File]::ReadAllText((Join-Path $Root 'Report.wer'))) 'report' 'ReportArchive')
$out['header_lines']=@(Write-LkeDumpHeader (Join-Path $Root 'good.dmp'))
$out['bad_lines']=@(Write-LkeDumpHeader (Join-Path $Root 'bad_first.dmp'))
function Get-WinEvent { @() }
$out['integration']=@(& (Join-Path $Root 'gate_state.ps1') -Mode wer)
$out | ConvertTo-Json -Depth 8 -Compress
''')
        done=subprocess.run([PWSH,'-NoProfile','-File',str(script),str(PS),str(root)],capture_output=True,text=True,timeout=60)
        if done.returncode: raise AssertionError(done.stderr+done.stdout)
        cls.data=json.loads(done.stdout)
    @classmethod
    def tearDownClass(cls): cls.tmp.cleanup()
    def test_signature_page_is_required(self): self.assertIn('refused',self.data['bad_first'])
    def test_signature_du64_is_required(self): self.assertIn('refused',self.data['bad_second'])
    def test_short_header_is_refused(self): self.assertIn('refused',self.data['short'])
    def test_truncated_parameters_are_refused(self): self.assertIn('refused',self.data['tiny'])
    def test_direct_short_parser_input_is_refused(self): self.assertIn('refused',self.data['direct4095'])
    def test_direct_tiny_parser_input_is_refused(self): self.assertIn('refused',self.data['direct96'])
    def test_c1_first_parameter_preserves_64_bits(self): self.assertEqual(self.data['c1']['p1'],'0x1122334455667788')
    def test_c1_preserves_all_four_parameters(self):
        self.assertEqual([self.data['c1']['p'+str(i)] for i in range(1,5)],['0x1122334455667788','0x0000000100000002','0xfedcba9876543210','0xffffffffffffffff'])
    def test_wer_line_includes_decoded_time(self): self.assertRegex(self.data['lines'][0],r'event_time="20[0-9-]+T[0-9:]+Z"')
    def test_wer_line_includes_bucket(self): self.assertIn('bucket="bucket=watchdog"',self.data['lines'][0])
    def test_collector_decodes_nested_reports(self):
        self.assertTrue(any(line.startswith('GATE_LKE report="deeper"') for line in self.data['integration']))
    def test_collector_does_not_silently_lose_a_report(self):
        self.assertEqual(len([line for line in self.data['integration'] if line.startswith('GATE_LKE report=')]),5)
    def test_collector_also_decodes_non_live_reports(self):
        self.assertTrue(any('P1="ordinary"' in line for line in self.data['integration']))
    def test_collector_reads_watchdog_header_in_place(self):
        self.assertTrue(any(line.startswith('GATE_LKE_HEADER bugcheck=0x00000193') for line in self.data['integration']))
    def test_real_code_label_is_decoded_as_p1(self): self.assertEqual(self.data['real_wer']['P1'],'193')
    def test_real_code_is_visible_in_the_wer_line(self): self.assertIn('P1="193"',self.data['real_lines'][0])
    def test_real_parameter_labels_use_their_names(self):
        self.assertEqual([self.data['real_wer']['P'+str(i)] for i in range(2,6)],['815','2','fffffa4efa4e0001','0'])
    def test_real_parameters_are_visible_in_the_wer_line(self):
        self.assertIn('P2="815" P3="2" P4="fffffa4efa4e0001" P5="0"',self.data['real_lines'][0])
    def test_real_os_product_labels_fill_p6_through_p8(self):
        self.assertEqual([self.data['real_wer']['P'+str(i)] for i in range(6,9)],['10_0_26100','0_0','256_1'])
    def test_real_shaped_header_matches_observed_parameters(self):
        self.assertEqual([self.data['real']['p'+str(i)] for i in range(1,5)],
                         ['0x0000000000000815','0x0000000000000002','0xfffffa4efa4e0001','0x0000000000000000'])
    def test_real_sized_dump_still_reads_only_the_header(self):
        self.assertEqual((self.data['real']['bugcheck'],self.data['real']['bytes_read']),('0x00000193',4096))
    def test_collector_reads_root_dump_not_only_watchdog(self):
        self.assertTrue(any('file="root.dmp"' in line for line in self.data['integration']))
    def test_collector_reads_other_nested_dump_names(self):
        self.assertTrue(any('file="other-name.dmp"' in line for line in self.data['integration']))
    def test_collector_reads_both_wer_stores(self):
        self.assertEqual(sum('file="same-name.dmp"' in line for line in self.data['integration']),2)
    def test_collector_header_count_includes_all_locations(self):
        self.assertIn('GATE_LKE_HEADERS files=5',self.data['integration'])
    def test_bad_second_layout_has_no_guessed_code(self): self.assertNotIn('bugcheck=',self.data['bad_second_lines'][0])
    def test_bad_second_layout_is_refused(self): self.assertIn('REFUSED',self.data['bad_second_lines'][0])
    def test_bugcheck_is_numeric_0x193(self): self.assertEqual(self.data['good']['bugcheck'],'0x00000193')
    def test_c1_is_not_relabelled_193(self): self.assertEqual(self.data['c1']['bugcheck'],'0x000000c1')
    def test_first_parameter_offset_and_width(self): self.assertEqual(self.data['good']['p1'],'0x0000000000000815')
    def test_second_parameter_is_not_truncated(self): self.assertEqual(self.data['good']['p2'],'0x0000000100000002')
    def test_third_parameter_preserves_high_bit(self): self.assertEqual(self.data['good']['p3'],'0xfedcba9876543210')
    def test_fourth_parameter_preserves_all_bits(self): self.assertEqual(self.data['good']['p4'],'0xffffffffffffffff')
    def test_reader_only_reads_0x1000_bytes(self): self.assertEqual(self.data['long']['bytes_read'],4096)
    def test_reader_preserves_header_with_extra_payload(self): self.assertEqual(self.data['long']['bugcheck'],'0x00000193')
    def test_wer_name_mapping_not_index_guess(self): self.assertEqual(self.data['wer']['P1'],'193')
    def test_wer_second_parameter_is_raw_not_guessed_hex(self): self.assertEqual(self.data['wer']['P2'],'815')
    def test_wer_all_eight_parameters_are_decoded(self): self.assertEqual([self.data['wer']['P'+str(i)] for i in range(5,9)],['5','6','7','8'])
    def test_wer_bucket_preserves_equals(self): self.assertEqual(self.data['wer']['bucket'],'bucket=watchdog')
    def test_wer_time_is_decoded(self): self.assertRegex(self.data['wer']['event_time'],r'^20[0-9-]+T[0-9:]+Z$')
    def test_bad_time_does_not_guess(self): self.assertEqual(self.data['missing']['event_time'],'unknown')
    def test_missing_parameter_does_not_become_zero(self): self.assertEqual(self.data['missing']['P1'],'-')
    def test_missing_named_value_does_not_borrow_another(self): self.assertEqual(self.data['partial']['P2'],'-')
    def test_wer_line_is_a_gate_lke_line(self): self.assertTrue(self.data['lines'][0].startswith('GATE_LKE '))
    def test_header_line_has_explicit_hex_code(self): self.assertIn('GATE_LKE_HEADER bugcheck=0x00000193',self.data['header_lines'][0])
    def test_invalid_header_has_no_guessed_bugcheck(self): self.assertNotIn('bugcheck=',self.data['bad_lines'][0])
    def test_invalid_header_is_explicitly_refused(self): self.assertIn('REFUSED',self.data['bad_lines'][0])

class GateIntegration(unittest.TestCase):
    gate=ROOT / 'scripts/bench/box/windows_gate.sh'
    def run_ratchet(self, die, count, mode=None, entries=None):
        text=self.gate.read_text(); start=text.index('lke_ratchet() {'); end=text.index('\n}\n',start)+3
        with tempfile.TemporaryDirectory() as tmp:
            script='\n'.join(['SOURCE='+str(ROOT), 'EV='+tmp, 'LKE_DIE='+die, 'LKE193='+str(count),
                'say() { echo "$*"; }', 'fail_op() { echo OPERATOR; exit 3; }',
                'fail_guest() { echo GUEST_FAIL; exit 1; }', text[start:end], 'lke_ratchet', 'echo CONTINUED'])
            env={'PATH':os.environ['PATH']}
            if mode is not None: env['KF_WINGATE_LKE_RATCHET']=mode
            if entries is not None:
                env['LKE193_ENTRIES']=str(entries); env['LKE193_UNRESOLVED']='0'
            done=subprocess.run(['bash','-c',script],env=env,capture_output=True,text=True,timeout=20)
            record=json.loads(Path(tmp,'lke193_ratchet.json').read_text())
        return done,record
    def test_gate_regression_fails_with_evidence(self):
        done,row=self.run_ratchet('AD102',3,'gate')
        self.assertEqual(done.returncode,1,done.stdout+done.stderr); self.assertEqual(row['verdict'],'FAIL')
        self.assertNotIn('CONTINUED',done.stdout)
    def test_gate_turing_regression_fails_with_evidence(self):
        done,row=self.run_ratchet('TU106',4,'gate')
        self.assertEqual(done.returncode,1,done.stdout+done.stderr); self.assertIn('GATE_LKE_RATCHET',done.stdout)
    def test_default_ad102_excess_warns_and_keeps_running(self):
        done,row=self.run_ratchet('AD102',3)
        self.assertEqual(done.returncode,0,done.stdout+done.stderr)
        self.assertEqual((row['count'],row['max_count'],row['verdict']),(3,1,'WARN'))
        self.assertIn('GATE_LKE_RATCHET die=AD102 count=3 max=1 verdict=WARN',done.stdout)
        self.assertIn('CONTINUED',done.stdout)
    def test_default_turing_excess_warns_and_keeps_running(self):
        done,row=self.run_ratchet('TU106',4)
        self.assertEqual(done.returncode,0,done.stdout+done.stderr)
        self.assertEqual((row['count'],row['max_count'],row['verdict']),(4,1,'WARN'))
        self.assertIn('GATE_LKE_RATCHET die=TU106 count=4 max=1 verdict=WARN',done.stdout)
    def test_explicit_report_mode_warns(self):
        done,row=self.run_ratchet('GA102',5,'report')
        self.assertEqual((done.returncode,row['verdict']),(0,'WARN'))
    def test_ad102_one_report_three_entries_does_not_trip_gate_mode(self):
        done,row=self.run_ratchet('AD102',1,'gate',entries=3)
        self.assertEqual((done.returncode,row['verdict'],row['entries']),(0,'PASS',3))
        self.assertIn('GATE_LKE_COUNTS reports=1 entries=3 unresolved=0 unit=distinct_reports',done.stdout)
    def test_turing_one_report_three_entries_does_not_trip_gate_mode(self):
        done,row=self.run_ratchet('TU106',1,'gate',entries=3)
        self.assertEqual((done.returncode,row['count'],row['entries']),(0,1,3))
        self.assertIn('entries=3',done.stdout)
    def test_gate_equal_max_keeps_running(self):
        done,row=self.run_ratchet('GA102',1)
        self.assertEqual(done.returncode,0,done.stdout+done.stderr); self.assertIn('CONTINUED',done.stdout)
    def test_gate_unknown_die_keeps_running_and_is_explicit(self):
        done,row=self.run_ratchet('GB206',50)
        self.assertEqual(done.returncode,0); self.assertEqual(row['verdict'],'UNKNOWN')
        self.assertIn('verdict=UNKNOWN',done.stdout)
    def test_existing_strict_mode_and_other_reset_rule_remain(self):
        text=self.gate.read_text()
        self.assertIn('strict="-Strict"',text)
        self.assertIn('^GATE_TDR .* total=0$',text)
        self.assertLess(text.index('lke_ratchet\n'),text.index("grep -aq '^GATE_TDR .* total=0$'"))
    def test_decode_module_is_staged_before_queries(self):
        text=self.gate.read_text()
        self.assertIn('guest_scp "$WIN/lke_decode.ps1"',text)
        self.assertIn(". (Join-Path $PSScriptRoot 'lke_decode.ps1')",(ROOT/'scripts/bench/windows/gate_state.ps1').read_text())
    def test_every_wer_is_decoded_before_the_old_timeline_filter(self):
        text=(ROOT/'scripts/bench/windows/gate_state.ps1').read_text()
        self.assertLess(text.index('Write-LkeWerLine $raw'),text.index("if ($raw -replace"))
        self.assertIn("-Filter '*.dmp'",text)
    def test_gate_streams_decoded_metadata_to_log(self):
        text=self.gate.read_text(); self.assertIn("grep -a '^GATE_LKE'",text)

class PostIdleRules(unittest.TestCase):
    """Execute the real post-idle shell checks against synthetic state/probe outputs."""
    def run_post_idle(self, *, die='AD102', count=3, strict=False, other=0, probe_bad=''):
        text=(ROOT/'scripts/bench/box/windows_gate.sh').read_text()
        start=text.index('lke_ratchet() {'); end=text.index('\n}\n',start)+3
        post=text[text.index('stage_begin post_idle\n'):text.index('stage_begin collect_stop\n')]
        with tempfile.TemporaryDirectory() as tmp:
            fake=Path(tmp,'winvm.sh'); fake.write_text('exit 0\n')
            script='\n'.join([
                'SOURCE='+str(ROOT), 'EV='+tmp, 'WV='+str(fake), 'TAG=synthetic', 'ENABLE_UTC=synthetic',
                'LKE_DIE='+die, 'COUNT='+str(count), 'OTHER='+str(other), 'BAD='+probe_bad,
                'say() { echo "$*"; }', 'stage_begin() { :; }', 'stage_end() { :; }',
                'smi() { return 0; }', 'probes_round() { PROBE_BAD=$BAD; }', 'wer_timeline() { :; }',
                'fail_op() { echo OPERATOR; exit 3; }', 'fail_guest() { echo GUEST_FAIL; exit 1; }',
                '''gps() {
                    if [[ "$*" == *"-Mode tdr"* ]]; then
                        total=$OTHER
                        for arg in "$@"; do [ "$arg" != -Strict ] || total=$((total+COUNT)); done
                        echo "GATE_TDR livekernelevent193=$COUNT livekernelevent_stale=0 livekernelevent_other=$OTHER total=$total"
                    else echo 'GATE_DISPLAY code=0 '; fi
                }''', text[start:end], post, 'echo POST_IDLE_PASSED'])
            env={'PATH':os.environ['PATH'],'KF_WINGATE_STRICT_LKE':'1' if strict else '0'}
            return subprocess.run(['bash','-c',script],env=env,capture_output=True,text=True,timeout=20)

    def test_default_ad102_excess_passes_only_zero_reset_checks(self):
        done=self.run_post_idle()
        self.assertEqual(done.returncode,0,done.stdout+done.stderr)
        self.assertIn('verdict=WARN',done.stdout)
        self.assertIn('POST_IDLE_PASSED',done.stdout)
    def test_default_turing_excess_passes_only_zero_reset_checks(self):
        done=self.run_post_idle(die='TU106',count=4)
        self.assertEqual(done.returncode,0,done.stdout+done.stderr)
        self.assertIn('verdict=WARN',done.stdout)
    def test_strict_still_gates_one_193_at_seeded_maximum(self):
        done=self.run_post_idle(count=1,strict=True)
        self.assertEqual(done.returncode,1,done.stdout+done.stderr)
        self.assertNotIn('POST_IDLE_PASSED',done.stdout)
    def test_strict_still_gates_193_on_an_unseeded_die(self):
        done=self.run_post_idle(die='GB206',count=1,strict=True)
        self.assertEqual(done.returncode,1,done.stdout+done.stderr)
    def test_non193_event_still_fails_with_a_warn_ratchet(self):
        done=self.run_post_idle(other=1)
        self.assertEqual(done.returncode,1,done.stdout+done.stderr)
        self.assertIn('verdict=WARN',done.stdout)
    def test_non193_events_still_fail_without_any_193(self):
        done=self.run_post_idle(count=0,other=2)
        self.assertEqual(done.returncode,1,done.stdout+done.stderr)
    def test_cuda_probe_failure_still_fails(self):
        done=self.run_post_idle(probe_bad='cup8')
        self.assertEqual(done.returncode,1,done.stdout+done.stderr)
    def test_d3d_probe_failure_still_fails(self):
        done=self.run_post_idle(probe_bad='d3d11')
        self.assertEqual(done.returncode,1,done.stdout+done.stderr)

class ReportFields(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        sys.path.insert(0,str(ROOT/'scripts/ci'))
        import vast_run_report
        cls.R=vast_run_report
    def build(self, die='AD102', count=3, maximum='2', verdict='FAIL'):
        return self.R.build_report([f'GATE_LKE_RATCHET die={die} count={count} max={maximum} verdict={verdict}',
              'WINDOWS_GATE_FAIL diag=KAYFABE stage=post_idle'],{})['windows']
    def test_failed_gate_still_reports_the_observed_count(self):
        self.assertEqual(self.build()['known_live_dumps_193'],3)
    def test_failed_gate_reports_ratchet_verdict(self):
        self.assertEqual(self.build()['live_dumps_193_ratchet'],'FAIL')
    def test_warn_survives_in_a_passing_gate_report(self):
        row=self.R.build_report(['GATE_LKE_RATCHET die=AD102 count=3 max=2 verdict=WARN',
            'WINDOWS_GATE_PASS resets=0 known_live_dumps_193=3'],{})['windows']
        self.assertEqual((row['known_live_dumps_193'],row['live_dumps_193_max'],row['live_dumps_193_ratchet']),(3,2,'WARN'))
    def test_turing_warn_retains_count_and_maximum(self):
        row=self.build('TU106',4,'3','WARN')
        self.assertEqual((row['known_live_dumps_193'],row['live_dumps_193_max'],row['live_dumps_193_ratchet']),(4,3,'WARN'))
    def test_report_keeps_distinct_reports_and_raw_entries(self):
        row=self.R.build_report(['GATE_LKE_COUNTS reports=1 entries=3 unresolved=0 unit=distinct_reports',
              'GATE_LKE_RATCHET die=AD102 count=1 max=1 verdict=PASS'],{})['windows']
        self.assertEqual((row['known_live_dumps_193'],row['live_dumps_193_entries'],row['live_dumps_193_unit']),(1,3,'distinct_reports'))
    def test_raw_entries_survive_on_a_warn_report(self):
        row=self.R.build_report(['GATE_LKE_COUNTS reports=2 entries=6 unresolved=0 unit=distinct_reports',
              'GATE_LKE_RATCHET die=TU106 count=2 max=1 verdict=WARN'],{})['windows']
        self.assertEqual((row['known_live_dumps_193'],row['live_dumps_193_entries'],row['live_dumps_193_ratchet']),(2,6,'WARN'))
    def test_unknown_raw_entry_count_is_not_invented(self):
        row=self.R.build_report(['GATE_LKE_COUNTS reports=1 entries=- unresolved=- unit=distinct_reports'],{})['windows']
        self.assertIsNone(row['live_dumps_193_entries'])
    def test_count_alias_is_preserved(self): self.assertEqual(self.build()['live_dumps_193'],3)
    def test_unknown_is_not_a_fabricated_pass(self):
        self.assertEqual(self.build('GB206',50,'-','UNKNOWN')['live_dumps_193_ratchet'],'UNKNOWN')
    def test_unreached_ratchet_has_no_invented_verdict(self):
        self.assertIsNone(self.R.build_report([],{})['windows']['live_dumps_193_ratchet'])
    def test_legacy_pass_still_populates_the_known_count(self):
        row=self.R.build_report(['WINDOWS_GATE_PASS resets=0 known_live_dumps_193=2'],{})['windows']
        self.assertEqual(row['known_live_dumps_193'],2)
    def test_malformed_ratchet_is_not_parsed(self):
        row=self.R.build_report(['GATE_LKE_RATCHET die=../AD102 count=3 max=2 verdict=PASS'],{})['windows']
        self.assertIsNone(row['live_dumps_193_ratchet'])

if __name__=='__main__': unittest.main()
