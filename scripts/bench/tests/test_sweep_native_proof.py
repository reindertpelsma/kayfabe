"""Offline proof that an optional native proof gates every guest entry in sweep.sh."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[3]
HELPER = REPO / 'scripts/drivermatrix/native_proof.sh'


class NativeProofTests(unittest.TestCase):
    def run_hook(self, prior='', result=0, retry='0', dry='0'):
        with tempfile.TemporaryDirectory() as tmp:
            env = dict(os.environ, TEST_PRIOR=prior, TEST_RESULT=str(result),
                       RETRY_FAILED=retry, DRY=dry, TEST_HELPER=str(HELPER), TEST_ROOT=tmp)
            script = r'''
set -uo pipefail
REPO=/trusted/revision DIR=$TEST_ROOT ARCH=TU106 H=595.91.07
SWEEP_NATIVE_PROOF=scripts/bench/diesweep/native_sweep_proof.sh
T_NATIVE_PROOF=600
row_rc() { printf '%s' "$TEST_PRIOR"; }
failed_before() { [ -n "$TEST_PRIOR" ] && [ "$TEST_PRIOR" != 0 ] && [ "${RETRY_FAILED:-0}" != 1 ]; }
row_start() { echo "START $1"; }
row_end() { echo "END $1 rc=$2"; TEST_PRIOR=$2; }
rowlog() { echo "$DIR/native.log"; }
step() { echo "EXEC $*"; return "$TEST_RESULT"; }
emit() { echo "$*"; }
source "$TEST_HELPER" || exit 98
if native_proof; then echo GUEST_ALLOWED; else echo GUEST_REFUSED; fi
'''
            p = subprocess.run(['bash', '-c', script], env=env, capture_output=True, text=True)
            self.assertEqual(p.returncode, 0, p.stderr)
            return p.stdout

    def test_current_failure_refuses_guest(self):
        self.assertIn('GUEST_REFUSED', self.run_hook(result=2))

    def test_current_timeout_refuses_guest(self):
        self.assertIn('GUEST_REFUSED', self.run_hook(result=124))

    def test_success_runs_pinned_hook_before_admission(self):
        out = self.run_hook()
        self.assertIn('EXEC 600 ', out)
        self.assertIn('bash /trusted/revision/scripts/bench/diesweep/native_sweep_proof.sh 595.91.07 TU106 /trusted/revision', out)
        self.assertLess(out.index('NATIVE_PROOF host='), out.index('GUEST_ALLOWED'))

    def test_old_success_is_remeasured_on_this_host(self):
        self.assertIn('EXEC ', self.run_hook(prior='0'))

    def test_old_success_cannot_hide_current_failure(self):
        self.assertIn('GUEST_REFUSED', self.run_hook(prior='0', result=2))

    def test_resumed_failure_stays_refused(self):
        self.assertIn('GUEST_REFUSED', self.run_hook(prior='2'))

    def test_resumed_timeout_stays_refused_without_reexecution(self):
        out = self.run_hook(prior='124')
        self.assertIn('GUEST_REFUSED', out)
        self.assertNotIn('EXEC ', out)

    def test_explicit_retry_measures_again(self):
        self.assertIn('EXEC ', self.run_hook(prior='2', retry='1'))

    def test_dry_run_never_executes(self):
        out = self.run_hook(dry='1')
        self.assertIn('PLAN row=native_proof:595.91.07', out)
        self.assertNotIn('EXEC ', out)

    def test_dry_run_of_previous_failure_still_shows_plan(self):
        out = self.run_hook(prior='2', dry='1')
        self.assertIn('PLAN row=native_proof:595.91.07', out)
        self.assertNotIn('EXEC ', out)

    def test_runner_gates_before_any_guest_step(self):
        text = (REPO / 'scripts/drivermatrix/sweep.sh').read_text()
        source = text.index('source "$REPO/scripts/drivermatrix/native_proof.sh"')
        hook = text.index('native_proof || abort')
        canary = text.index('if [ "${#thin[@]}" -gt 0 ] && row_begin "canary:')
        self.assertLess(source, hook)
        self.assertLess(hook, canary)

    def test_real_dry_plan_orders_proof_before_canary(self):
        env = dict(os.environ, DRY_RUN='1', SWEEP_DRY_ARCH='TU106',
                   TAG='s4offline', HOSTS='595.91.07', GUESTS='580.159.04',
                   REF_HOST='595.91.07', DEFAULT_GUEST='580.159.04', MIXED='',
                   SWEEP_NATIVE_PROOF='scripts/bench/diesweep/native_sweep_proof.sh')
        p = subprocess.run(['bash', str(REPO / 'scripts/drivermatrix/sweep.sh')],
                           env=env, capture_output=True, text=True, timeout=20)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertLess(p.stdout.index('PLAN row=native_proof:'), p.stdout.index('PLAN row=canary:'))


if __name__ == '__main__':
    unittest.main()
