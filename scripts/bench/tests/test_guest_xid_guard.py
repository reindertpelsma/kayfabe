#!/usr/bin/env python3
"""S2 EXPECTED RED: the approved guest-control guard is absent.

INFERRED hypothesis: exact guest proof can extend the frozen native baseline.
FALSIFIER: any non-source-matched fault is accepted, any required proof can be
removed, or an approved complete proof cannot pass. Synthetic missing-page
records below are parser fixtures, never historical hardware evidence.
"""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('guest_guard', ROOT / 'scripts/bench/native_xid_guard.py')
G = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(G)
FIX = ROOT / 'traces/sweep_windows_20261021/lease_57081143'


class GuestGuardTests(unittest.TestCase):
    def setUp(self):
        self.base = json.loads((FIX / 'native/native_xid_baseline.json').read_text())['approved_native_xid_lines']
        self.lines = (FIX / 'linux_stop/kernel_xids.log').read_text().splitlines()[:9]
        self.serial = (FIX / 'linux_stop/fast_sw2ad106d1_595_defer-liveness_serial.log.projection.txt').read_text()
        self.qlog = (FIX / 'linux_stop/fast_sw2ad106d1_595_defer-liveness_qemu.log.projection.txt').read_text()
        self.context = {'pid': 91947, 'starttime': 12345, 'exe': '/test/qemu-system-x86_64',
                        'arm': '--defer-liveness', 'revision': 'a' * 40,
                        'source_pin': {'raw': 'b' * 64, 'runner': 'c' * 64}}

    def approve(self, *, lines=None, serial=None, qlog=None, context=None, final=True):
        return G.approve_guest('\n'.join(self.lines if lines is None else lines) + '\n', self.base,
                               self.serial if serial is None else serial,
                               self.qlog if qlog is None else qlog,
                               self.context if context is None else context, final=final)

    def missing(self):
        context = dict(self.context, pid=92940, arm='--missing-page-fault')
        line = (FIX / 'linux_stop/kernel_xids.log').read_text().splitlines()[9]
        serial = '''FASTGUEST: arms --missing-page-fault --probe-launch-dma  deadline 166s
info  R3 bystander before = Landed
FAULT_MARK=victim_release va=0x0000000a11000000
kayfabe-isolate: DOORBELL-STORE #1 host_token=0x00000004 runlist=0 chid=4
info  R3 victim           = Lost
info  R3 notifier         = fired=true status=0xffff except_type=0x1f engine=0x0001
info  R3 bystander after  = Landed
RUNGCTL_missing_page=PASS
RUNG_missing_page=PASS
FASTGUEST: client rc=0 at 25.40s
FASTGUEST: DONE
'''
        qlog = 'kf3: RC host twin 0x1c (guest chid 0x4, engine 0x9) wrote its notifier: except_type=0x1f (Xid 31) — forwarding RC_TRIGGERED\n'
        return dict(lines=self.base + [line], serial=serial, qlog=qlog, context=context)

    def test_complete_measured_defer_proof_accepts_exact_four(self):
        self.assertEqual(self.approve(), tuple(self.lines))

    def test_source_shaped_missing_page_fixture_accepts_exact_one(self):
        case = self.missing()
        self.assertEqual(self.approve(**case), tuple(case['lines']))

    def test_qemu_pid_is_exact_defer(self):
        self.assert_refused(lines=[l.replace('pid=91947', 'pid=91948') for l in self.lines])

    def test_qemu_pid_is_exact_missing(self):
        c = self.missing(); c['context']['pid'] += 1; self.assert_refused(**c)

    def test_guest_host_twin_is_exact_defer(self):
        self.assert_refused(qlog=self.qlog.replace('twin 0x1b', 'twin 0x2b'))

    def test_guest_host_twin_is_exact_missing(self):
        c = self.missing(); c['qlog'] = c['qlog'].replace('twin 0x1c', 'twin 0x2c'); self.assert_refused(**c)

    def test_guest_channel_is_exact_defer(self):
        self.assert_refused(qlog=self.qlog.replace('guest chid 0x3', 'guest chid 0x7'))

    def test_guest_channel_is_exact_missing(self):
        c = self.missing(); c['qlog'] = c['qlog'].replace('guest chid 0x4', 'guest chid 0x7'); self.assert_refused(**c)

    def test_fault_address_is_exact_defer(self):
        self.assert_refused(lines=[l.replace('0x60_00010000', '0x60_00020000') if 'qemu-system' in l else l for l in self.lines])

    def test_fault_address_is_exact_missing(self):
        c = self.missing(); c['serial'] = c['serial'].replace('a11000000', 'a12000000'); self.assert_refused(**c)

    def test_xid32_fatal_defer(self):
        self.assert_refused(lines=self.lines[:5] + [l.replace(': 31,', ': 32,') for l in self.lines[5:]])

    def test_xid32_fatal_missing(self):
        c = self.missing(); c['lines'][-1] = c['lines'][-1].replace(': 31,', ': 32,'); self.assert_refused(**c)

    def test_fault_type_is_exact_defer(self):
        self.assert_refused(lines=self.lines[:5] + [l.replace('FAULT_PTE', 'FAULT_PDE') for l in self.lines[5:]])

    def test_fault_type_is_exact_missing(self):
        c = self.missing(); c['lines'][-1] = c['lines'][-1].replace('VIRT_WRITE', 'VIRT_READ'); self.assert_refused(**c)

    def test_recovery_required_defer(self):
        self.assert_refused(serial=self.serial.replace('DL rescue copy     = Live', 'DL rescue copy     = Dead'))

    def test_recovery_required_missing(self):
        c = self.missing(); c['serial'] = c['serial'].replace('bystander after  = Landed', 'bystander after  = Lost'); self.assert_refused(**c)

    def test_native_history_immutable_defer(self):
        self.assert_refused(lines=[l.replace('pid=90242', 'pid=90243') for l in self.lines])

    def test_native_history_immutable_missing(self):
        c = self.missing(); c['lines'][0] += ' changed'; self.assert_refused(**c)

    def test_duplicate_fault_fatal_defer(self):
        self.assert_refused(lines=self.lines + [self.lines[-1]])

    def test_duplicate_fault_fatal_missing(self):
        c = self.missing(); c['lines'] += [c['lines'][-1]]; self.assert_refused(**c)

    def test_missing_source_marker_fatal_defer(self):
        self.assert_refused(serial=self.serial.replace('FAULT_MARK=', 'NO_MARK='))

    def test_missing_source_marker_fatal_missing(self):
        c = self.missing(); c['serial'] = c['serial'].replace('FAULT_MARK=', 'NO_MARK='); self.assert_refused(**c)

    def test_unrelated_pci_fatal_defer(self):
        self.assert_refused(lines=self.lines[:5] + [l.replace('00:07', '00:08') for l in self.lines[5:]])

    def test_unrelated_pci_fatal_missing(self):
        c = self.missing(); c['lines'][-1] = c['lines'][-1].replace('00:07', '00:08'); self.assert_refused(**c)

    def test_unrelated_route_fatal_defer(self):
        self.assert_refused(lines=self.lines[:5] + [l.replace('CE0', 'GR0_PBDMA0') for l in self.lines[5:]])

    def test_unrelated_route_fatal_missing(self):
        c = self.missing(); c['lines'][-1] = c['lines'][-1].replace('HUBCLIENT_CE1', 'HUBCLIENT_ESC'); self.assert_refused(**c)

    def test_pid_zero_fatal(self):
        c = dict(self.context, pid=0); self.assert_refused(context=c)

    def test_pid_one_fatal(self):
        c = dict(self.context, pid=1); self.assert_refused(context=c)

    def test_starttime_required(self):
        self.assert_refused(context=dict(self.context, starttime=0))

    def test_source_pin_required(self):
        self.assert_refused(context=dict(self.context, source_pin={}))

    def test_revision_required(self):
        self.assert_refused(context=dict(self.context, revision=''))

    def test_unknown_arm_refused(self):
        self.assert_refused(context=dict(self.context, arm='--timer'))

    def test_multiple_markers_same_journal_record_fatal(self):
        self.assert_refused(lines=self.lines[:5] + [self.lines[5] + ' ' + self.lines[5]] + self.lines[6:])

    def test_trailing_unparsed_xid_payload_fatal(self):
        self.assert_refused(lines=self.lines[:8] + [self.lines[8] + ' unknown'])

    def test_trailing_unparsed_missing_page_payload_fatal(self):
        c = self.missing(); c['lines'][-1] += ' unknown'; self.assert_refused(**c)

    def test_hidden_first_xid_record_fatal_defer(self):
        self.assert_refused(lines=self.lines[:5] + ['NVRM: Xid unknown first record ' + self.lines[5]] + self.lines[6:])

    def test_hidden_first_xid_record_fatal_missing(self):
        c = self.missing(); c['lines'][-1] = 'NVRM: Xid unknown first record ' + c['lines'][-1]; self.assert_refused(**c)

    def test_incomplete_missing_page_correlation_is_never_approved(self):
        c = self.missing(); c['qlog'] = ''; self.assert_refused(**c)

    def test_missing_defer_correlation_is_never_approved(self):
        self.assert_refused(qlog='')

    def test_no_positive_verdict_no_approval_defer(self):
        self.assert_refused(serial=self.serial.replace('RUNG_defer_liveness=PASS', 'RUNG_defer_liveness=FAIL'))

    def test_no_positive_verdict_no_approval_missing(self):
        c = self.missing(); c['serial'] = c['serial'].replace('RUNG_missing_page=PASS', 'RUNG_missing_page=FAIL'); self.assert_refused(**c)

    def test_wrong_primitive_refused(self):
        c = self.missing(); c['serial'] = c['serial'].replace('--probe-launch-dma', '--probe-sem-release'); self.assert_refused(**c)

    def test_running_faults_are_pending_never_baseline(self):
        serial = self.serial.split('info  DL prime copy', 1)[0]
        self.assertIsNone(self.approve(lines=self.lines[:6], serial=serial, qlog='', final=False))

    def missing_crlf(self, final=True):
        case = self.missing(); case['serial'] = case['serial'].replace('\n', '\r\n')
        if not final: case['serial'] = case['serial'].split('info  R3 victim',1)[0]
        return case

    def test_live_missing_page_CRLF_marker_matches_pending_source_operation(self):
        case = self.missing_crlf(False)
        refusal = None; result = 'not-run'
        try: result = self.approve(**case, final=False)
        except G.GuardRefused as error: refusal = error
        self.assertIsNone(refusal); self.assertIsNone(result)

    def test_complete_missing_page_CRLF_proof_accepts_exact_one(self):
        case = self.missing_crlf(); refusal = None; result = None
        try: result = self.approve(**case)
        except G.GuardRefused as error: refusal = error
        self.assertIsNone(refusal); self.assertEqual(result,tuple(case['lines']))

    def test_live_CR_inside_address_is_not_line_framing(self):
        case = self.missing_crlf(False); case['serial'] = case['serial'].replace('a11000000','a1100\r0000')
        self.assert_refused(**case, final=False)

    def test_complete_CR_inside_address_is_not_line_framing(self):
        case = self.missing_crlf(); case['serial'] = case['serial'].replace('a11000000','a1100\r0000')
        self.assert_refused(**case)

    def test_CRLF_missing_page_wrong_address_still_fatal(self):
        case = self.missing_crlf(); case['serial'] = case['serial'].replace('a11000000','a12000000'); self.assert_refused(**case)

    def test_CRLF_missing_page_wrong_pid_still_fatal(self):
        case = self.missing_crlf(); case['context']['pid'] += 1; self.assert_refused(**case)

    def test_running_unknown_address_is_fatal(self):
        serial = self.serial.split('info  DL prime copy', 1)[0]
        self.assert_refused(lines=self.lines[:5] + [self.lines[5].replace('0x60_00010000', '0x60_00020000')], serial=serial, qlog='', final=False)

    def test_running_xid32_is_fatal(self):
        self.assert_refused(lines=self.lines[:5] + [self.lines[5].replace(': 31,', ': 32,')], final=False)

    def test_final_requires_every_expected_fault(self):
        self.assert_refused(lines=self.lines[:8])

    def test_final_requires_missing_page_fault_itself(self):
        c = self.missing(); c['lines'] = self.base; self.assert_refused(**c)

    # Blackwell-shaped twins. kf3 prints the channel's work-submit TOKEN as `host_token`
    # (RUNLIST_DOORBELL 30 | RUNLIST_ID 22:16 | VECTOR 11:0, kf-trap tokenindex.rs; bit 30 on every
    # GB20x token); the host Xid record names the bare chid. GB203 gate 2026-11-10 measured guest
    # chid 0x3 -> `host 0x4000000a` (log), and the native arm's Xid channel equalled its token's
    # chid. The shape below remaps the Ada fixture's host chids 0x1b/0x1c to 0xa/0xb; that the
    # guest arm's host Xid then says 0xa/0xb is INFERRED (the log has no guest-arm Xid record).
    def blackwell(self, token_bits=1 << 30):
        lines = self.lines[:5] + [
            l.replace('channel 0x0000001b', 'channel 0x0000000a').replace('channel 0x0000001c', 'channel 0x0000000b')
            for l in self.lines[5:]]
        qlog = self.qlog
        for old, chid in (('0x1b', 0xa), ('0x1c', 0xb)):
            qlog = qlog.replace('twin %s ' % old, 'twin %#x ' % (token_bits | chid))
        return lines, qlog

    def first_twin(self, qlog):
        return [l for l in qlog.splitlines() if 'RC host twin' in l][0] + '\n'

    def prime_only_serial(self):
        return self.serial.split('info  DL prime copy', 1)[0]

    def test_blackwell_token_bit30_accepts_at_final(self):
        lines, qlog = self.blackwell()
        self.assertIn('twin 0x4000000a ', qlog)
        self.assertEqual(self.approve(lines=lines, qlog=qlog), tuple(lines))

    def test_blackwell_token_is_pending_at_nonfinal(self):
        lines, qlog = self.blackwell()
        self.assertIsNone(self.approve(lines=lines[:6], serial=self.prime_only_serial(),
                                       qlog=self.first_twin(qlog), final=False))

    def test_blackwell_nonfinal_wrong_chid_refused(self):
        lines, qlog = self.blackwell()
        twin = self.first_twin(qlog).replace('0x4000000a', '0x4000002a')
        self.assert_refused(lines=lines[:6], serial=self.prime_only_serial(), qlog=twin, final=False)

    def test_blackwell_wrong_host_chid_refused(self):
        lines, qlog = self.blackwell()
        self.assert_refused(lines=lines, qlog=qlog.replace('twin 0x4000000a', 'twin 0x4000002a'))

    def test_blackwell_wrong_guest_chid_refused(self):
        lines, qlog = self.blackwell()
        self.assert_refused(lines=lines, qlog=qlog.replace('guest chid 0x3', 'guest chid 0x7'))

    def test_blackwell_bit30_alone_is_not_a_chid(self):
        lines, qlog = self.blackwell()
        self.assert_refused(lines=lines, qlog=qlog.replace('twin 0x4000000a', 'twin 0x40000000'))

    def test_runlist_bits_in_token_do_not_change_the_chid(self):
        # RUNLIST_ID 22:16 = 2, with and without bit 30 (non-GR engines carry a runlist before Blackwell).
        for bits in (2 << 16, (2 << 16) | (1 << 30)):
            lines, qlog = self.blackwell(bits)
            self.assertEqual(self.approve(lines=lines, qlog=qlog), tuple(lines), hex(bits))

    def test_runlist_shaped_wrong_chid_refused(self):
        lines, qlog = self.blackwell(2 << 16)
        self.assertIn('twin 0x2000a ', qlog)
        self.assert_refused(lines=lines, qlog=qlog.replace('twin 0x2000a', 'twin 0x2002a'))

    def test_token_reserved_bits_refused(self):
        # Reserved: bit 31, bits 29:23, bits 15:12. The chid field is right but the token is not a token.
        for junk in (1 << 31, 1 << 29, 1 << 23, 1 << 15, 1 << 12, 0xF000, 0x3F800000):
            lines, qlog = self.blackwell(junk | (1 << 30))
            self.assert_refused(lines=lines, qlog=qlog)
            self.assert_refused(qlog=self.qlog.replace('twin 0x1b ', 'twin %#x ' % (junk | 0x1b)))
        # A reserved bit on a twin whose Xid has not arrived is refused too, never pending.
        lines, qlog = self.blackwell()
        twin = self.first_twin(qlog).replace('0x4000000a', '0xc000000a')
        self.assert_refused(lines=lines[:5], serial=self.prime_only_serial(), qlog=twin, final=False)

    def test_blackwell_missing_page_token_accepts(self):
        c = self.missing()
        c['lines'][-1] = c['lines'][-1].replace('channel 0x0000001c', 'channel 0x0000000c')
        c['qlog'] = c['qlog'].replace('twin 0x1c', 'twin 0x4000000c')
        self.assertEqual(self.approve(**c), tuple(c['lines']))
        c['qlog'] = c['qlog'].replace('twin 0x4000000c', 'twin 0x4000002c')
        self.assert_refused(**c)

    def test_token_chid_helper_layout(self):
        self.assertEqual(G.token_chid(0x1b), 0x1b)
        self.assertEqual(G.token_chid(0x4000000a), 0xa)
        self.assertEqual(G.token_chid(0x40020fff), 0xfff)
        with self.assertRaises(G.GuardRefused):
            G.token_chid(0x8000000a)

    def assert_refused(self, **kwargs):
        with self.assertRaises(G.GuardRefused):
            self.approve(**kwargs)


class PinTests(unittest.TestCase):
    def test_current_suite_pin_is_recorded(self):
        rev = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        pin = G.pin_suite(ROOT, rev)
        self.assertGreaterEqual(len(pin), 4)

    def test_stale_source_revision_fatal(self):
        with self.assertRaises(G.GuardRefused):
            G.pin_suite(ROOT, '0' * 40)

    def test_other_valid_format_revision_fatal(self):
        with self.assertRaises(G.GuardRefused):
            G.pin_suite(ROOT, 'f' * 40)

    def changed_source(self, selected):
        from unittest import mock
        original = Path.read_bytes
        rev = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        def read(path):
            data = original(path)
            return data + b'\nchanged\n' if str(path).endswith(selected) else data
        with mock.patch.object(Path, 'read_bytes', read):
            with self.assertRaises(G.GuardRefused): G.pin_suite(ROOT, rev)

    def test_changed_raw_source_refused(self):
        self.changed_source('crates/kayfabe-rm-ladder/src/main.rs')

    def test_changed_runner_source_refused(self):
        self.changed_source('scripts/fastguest/run_fast_guest.sh')

    def test_missing_source_revision_fatal(self):
        with self.assertRaises(G.GuardRefused):
            G.pin_suite(ROOT, '')


class NativeExactFiveTests(unittest.TestCase):
    """FALSIFIER: a changed four-record native shape is accepted as baseline."""
    def check_four(self, folder, prefix, journal):
        defer = (folder / (prefix + '_defer-liveness.log')).read_text()
        missing = (folder / (prefix + '_missing-page-fault.log')).read_text()
        # Synthetic exec attestations keep this test about the four-fault count.
        pids = (89396, 89433) if '1ce11cf4' in prefix else (90242, 90282)
        defer = f'KF_NATIVE_EXEC pid={pids[0]} name=kayfabe-rm-ladd\n' + defer
        missing = f'KF_NATIVE_EXEC pid={pids[1]} name=kayfabe-rm-ladd\n' + missing
        defer = defer.replace('DL test copy       = Dead', 'DL test copy       = Live')
        lines = list(G.xid_lines(journal)); del lines[2]
        with self.assertRaises(G.GuardRefused):
            G.approve_native('\n'.join(lines), defer, missing)

    def test_prior_lease_cannot_approve_four_native_faults(self):
        folder = ROOT / 'traces/sweep_windows_20261021/lease_57064037/native'
        self.check_four(folder, 'sweep1ce11cf4_ad106_native595', (folder / 'native_expected_xids.log').read_text())

    def test_second_lease_cannot_approve_four_native_faults(self):
        folder = FIX / 'native'
        journal = '\n'.join(json.loads((folder / 'native_xid_baseline.json').read_text())['approved_native_xid_lines'])
        self.check_four(folder, 'sweep74aa78ab_ad106_native595', journal)


if __name__ == '__main__':
    unittest.main()
