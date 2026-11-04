#!/usr/bin/env python3
"""GPU-free harness contracts: evaluate assignments/log summaries, never launch a VM."""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
WR = HERE / 'host_tools/runners/wr-run.sh'


def exported_flags(path, extra='', win_flags=None):
    line = next(line for line in path.read_text().splitlines()
                if line.startswith('export WIN_FLAGS='))
    env = {'PATH': os.defpath, 'EXTRA': extra}
    if win_flags is not None:
        env['WIN_FLAGS'] = win_flags
    result = subprocess.run(['bash', '-eu', '-c', line + '\nprintf "%s" "$WIN_FLAGS"'],
                            env=env, capture_output=True, text=True, check=True)
    return result.stdout.split()


def summary(log):
    line = next(line for line in WR.read_text().splitlines()
                if line.startswith('L "experiments:'))
    with tempfile.NamedTemporaryFile(mode='w') as f:
        f.write(log)
        f.flush()
        result = subprocess.run(['bash', '-u', '-c',
                                 'L(){ printf "%s\\n" "$*"; }; ' + line],
                                env={'PATH': os.defpath, 'LOG': f.name},
                                capture_output=True, text=True, check=True)
    match = re.search(r'experiments: (\d+) mirror, (\d+) ilut-unit', result.stdout)
    if not match:
        raise AssertionError(result.stdout)
    return tuple(map(int, match.groups()))


class TraceConsumers(unittest.TestCase):
    def test_winpass_default_requests_the_rpc_marker_it_waits_for(self):
        self.assertIn('KF3_RPC_TRACE=1', exported_flags(HERE / 'winpass_cycle.sh'))

    def test_winpass_empty_override_still_has_a_marker_producer(self):
        self.assertIn('KF3_RPC_TRACE=1', exported_flags(HERE / 'winpass_cycle.sh', win_flags=''))

    def test_winpass_preserves_explicit_research_override(self):
        self.assertEqual(exported_flags(HERE / 'winpass_cycle.sh', win_flags='KF3_PROF=1'),
                         ['KF3_PROF=1'])

    def test_wr_default_requests_rpc_for_its_stall_and_census(self):
        self.assertIn('KF3_RPC_TRACE=1', exported_flags(WR))

    def test_wr_extra_flags_do_not_remove_its_rpc_marker(self):
        flags = exported_flags(WR, extra='KF3_PROF=1')
        self.assertIn('KF3_RPC_TRACE=1', flags)
        self.assertIn('KF3_PROF=1', flags)

    def test_wr_counts_current_decoder_selection_logs(self):
        self.assertEqual(summary('kf3: display: MIRROR accepted on DIRECT ILUT/OLUT where no negative input can reach it (KF3_DISPLAY_LUT_MIRROR=0 selects the old arm)\n'
                                 'kf3: display: SET_OFFSET_ILUT read in 256-byte units, as OLUT/TMO (KF3_DISPLAY_ILUT_OFFSET_256=0 selects the old byte-offset arm)\n'),
                         (1, 1))

    def test_wr_counts_each_current_selection_once_and_ignores_other_logs(self):
        mirror = 'kf3: display: MIRROR accepted on DIRECT ILUT/OLUT (KF3_DISPLAY_LUT_MIRROR=0 selects the old arm)\n'
        units = 'kf3: display: SET_OFFSET_ILUT read in 256-byte units (KF3_DISPLAY_ILUT_OFFSET_256=0 selects the old byte-offset arm)\n'
        self.assertEqual(summary('unrelated\n' + mirror * 2 + units * 3), (2, 3))

    def test_wr_keeps_historical_experiment_log_support(self):
        self.assertEqual(summary('kf3: display: EXPERIMENT KF3_DISPLAY_LUT_MIRROR=1 old\n'
                                 'kf3: display: EXPERIMENT KF3_DISPLAY_ILUT_OFFSET_256=1 old\n'),
                         (1, 1))


if __name__ == '__main__':
    unittest.main()
