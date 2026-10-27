"""OWNER-APPROVED 2026-10-22: CE family + exact source, never a die table."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]

def load(name, relative):
    spec = importlib.util.spec_from_file_location(name, ROOT / relative)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

R = load('round2_replay', 'scripts/bench/tests/test_native_xid_route_round2.py')
J = load('guest_fixture', 'scripts/bench/tests/test_guest_xid_guard.py')
G = R.G


class OwnerCopyFamilyRule(unittest.TestCase):
    def native(self, die, change=lambda x: x):
        journal, defer, missing = R.fixtures(die)
        changed = change(journal)
        return G.approve_native(changed, defer, missing), G.xid_lines(changed)

    def test_authentic_ad102_exact_five_admit(self):
        actual, expected = self.native('AD102')
        self.assertEqual(len(actual), 5)
        self.assertEqual(actual, expected)

    def test_authentic_ga102_four_ce_plus_gr_admit(self):
        actual, expected = self.native('GA102')
        self.assertEqual(sum('ENGINE CE0' in x for x in actual), 4)
        self.assertEqual(actual, expected)

    def test_ad102_ce0_parser_fixture_admits_exact_shapes(self):
        self.assertEqual(*self.native('AD102', lambda x: x.replace('HUBCLIENT_CE1', 'HUBCLIENT_CE0')))

    def test_native_ce1_wrong_channel_refuses(self):
        with self.assertRaises(G.GuardRefused):
            self.native('GA102', lambda x: x.replace('HUBCLIENT_CE0', 'HUBCLIENT_CE1').replace('channel 0x00000004', 'channel 0x00000024', 1))

    def test_native_ce1_wrong_va_refuses(self):
        with self.assertRaises(G.GuardRefused):
            self.native('GA102', lambda x: x.replace('HUBCLIENT_CE0', 'HUBCLIENT_CE1').replace('0x60_00010000', '0x60_00020000', 1))

    def test_native_extra_record_refuses(self):
        with self.assertRaises(G.GuardRefused):
            self.native('GA102', lambda x: x + G.xid_lines(x)[0] + '\n')

    def test_native_non_ce_refuses(self):
        with self.assertRaises(G.GuardRefused):
            self.native('GA102', lambda x: x.replace('HUBCLIENT_CE0', 'HUBCLIENT_PMU', 1))

    def test_native_ce_suffix_garbage_refuses(self):
        with self.assertRaises(G.GuardRefused):
            self.native('GA102', lambda x: x.replace('HUBCLIENT_CE0', 'HUBCLIENT_CE0_UNKNOWN', 1))

    def test_native_wrong_type_refuses(self):
        with self.assertRaises(G.GuardRefused):
            self.native('GA102', lambda x: x.replace('FAULT_PTE', 'FAULT_PDE', 1))

    def guest(self):
        case = J.GuestGuardTests('test_complete_measured_defer_proof_accepts_exact_four')
        case.setUp()
        return case

    def test_guest_defer_ce0_with_exact_source_admits(self):
        case = self.guest()
        lines = case.lines[:5] + [x.replace('HUBCLIENT_CE1', 'HUBCLIENT_CE0') for x in case.lines[5:]]
        self.assertEqual(case.approve(lines=lines), tuple(lines))

    def test_guest_missing_ce0_with_exact_source_admits(self):
        case = self.guest(); values = case.missing()
        values['lines'][-1] = values['lines'][-1].replace('HUBCLIENT_CE1', 'HUBCLIENT_CE0')
        self.assertEqual(case.approve(**values), tuple(values['lines']))

    def test_guest_non_ce_refuses_defer(self):
        case = self.guest()
        lines = case.lines[:5] + [x.replace('HUBCLIENT_CE1', 'HUBCLIENT_PMU') for x in case.lines[5:]]
        with self.assertRaises(J.G.GuardRefused):case.approve(lines=lines)

    def test_guest_non_ce_refuses_missing(self):
        case = self.guest(); values = case.missing()
        values['lines'][-1] = values['lines'][-1].replace('HUBCLIENT_CE1', 'HUBCLIENT_PMU')
        with self.assertRaises(J.G.GuardRefused):case.approve(**values)


if __name__ == '__main__':
    unittest.main()
