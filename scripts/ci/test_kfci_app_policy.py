"""Falsifier: Python apps-gate policy admits UVM, hides metadata or widens child env."""

import io
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from kfci.apps_gate import AppsGate
from kfci.proc import BASE_ENV, ProcessResult
from kfci.step import Context, RetryClass


POLICY = 'APP_POLICY name=attach_verify os=linux result=NOT_RUN note=unsupported (UVM)\n'


class PythonAppPolicy(unittest.TestCase):
    def context(self, expected='vectorAdd', policy=None, real_helper=False):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        listing = root / 'list'
        listing.write_text(expected + '\n')
        ctx = Context(parameters={'tag': 'policy', 'prov': str(root / 'prov')},
                      env={'PATH': os.environ['PATH'], 'KF_APPS_LIST': str(listing),
                           'KF_APPS_ROOT': str(root / 'apps'), 'UNLISTED': 'sentinel',
                           'KF3_UNIT_CONFIG': 'sentinel'},
                      output=io.StringIO(), deadline=time.monotonic() + 20, root=root)
        actual_process = ctx.process
        calls = []
        if real_helper:
            helper = root / 'scripts/ci/app_policy.py'
            helper.parent.mkdir(parents=True)
            helper.write_text('import os,sys\n'
                              'if "KF3_UNIT_CONFIG" in os.environ or "UNLISTED" in os.environ: sys.exit(1)\n'
                              'print(' + repr(POLICY.rstrip('\n')) + ')\n')

        def process(argv, **kw):
            calls.append((argv, kw))
            if argv[0] == sys.executable:
                return actual_process(argv, **kw) if real_helper else (
                    policy if policy is not None else ProcessResult(0, POLICY))
            self.assertEqual(argv[-1], 'list', 'preflight must never start a matrix/provisioner')
            return ProcessResult(0, 'vectorAdd deviceQuery attach_verify UnifiedMemoryStreams\n')

        ctx.process = process
        return ctx, calls

    def test_policy_helper_uses_isolated_python_and_base_environment(self):
        ctx, calls = self.context()
        result = AppsGate().run(ctx)
        self.assertEqual(result.retry_class, RetryClass.NOT_BUILT)
        argv, options = calls[0]
        self.assertEqual(argv, [sys.executable, '-I', str(ctx.root / 'scripts/ci/app_policy.py'), 'linux'])
        self.assertEqual(options['allow_env'], BASE_ENV)
        self.assertNotIn('KF3_UNIT_CONFIG', options['allow_env'])

    def test_policy_helper_cannot_receive_legacy_or_unlisted_configuration(self):
        ctx, _ = self.context(real_helper=True)
        result = AppsGate().run(ctx)
        self.assertEqual(result.retry_class, RetryClass.NOT_BUILT, ctx.output.getvalue())
        self.assertIn(POLICY, ctx.output.getvalue())

    def test_failed_policy_helper_is_bad_list(self):
        ctx, _ = self.context(policy=ProcessResult(1, POLICY))
        self.assertEqual(AppsGate().run(ctx).retry_class, RetryClass.BAD_LIST)
        self.assertNotIn('APPS_STAGE_LIST', ctx.output.getvalue())

    def test_truncated_policy_helper_is_bad_list(self):
        ctx, _ = self.context(policy=ProcessResult(0, POLICY, truncated=True))
        self.assertEqual(AppsGate().run(ctx).retry_class, RetryClass.BAD_LIST)

    def test_timed_out_policy_helper_is_bad_list(self):
        ctx, _ = self.context(policy=ProcessResult(0, POLICY, timed_out=True))
        self.assertEqual(AppsGate().run(ctx).retry_class, RetryClass.BAD_LIST)

    def test_missing_policy_metadata_is_bad_list(self):
        ctx, _ = self.context(policy=ProcessResult(0, ''))
        self.assertEqual(AppsGate().run(ctx).retry_class, RetryClass.BAD_LIST)

    def test_empty_policy_cannot_admit_unsupported_override(self):
        ctx, _ = self.context(expected='attach_verify', policy=ProcessResult(0, ''))
        self.assertEqual(AppsGate().run(ctx).retry_class, RetryClass.BAD_LIST)

    def test_malformed_policy_metadata_is_bad_list(self):
        ctx, _ = self.context(policy=ProcessResult(0, 'APP_POLICY name=a os=linux result=PASS note=wrong\n'))
        self.assertEqual(AppsGate().run(ctx).retry_class, RetryClass.BAD_LIST)
        self.assertNotIn('result=PASS', ctx.output.getvalue())

    def test_unknown_gate_name_is_bad_list(self):
        ctx, _ = self.context(expected='not_known')
        self.assertEqual(AppsGate().run(ctx).retry_class, RetryClass.BAD_LIST)

    def test_another_unknown_gate_name_is_bad_list(self):
        ctx, _ = self.context(expected='not_known_either')
        self.assertEqual(AppsGate().run(ctx).retry_class, RetryClass.BAD_LIST)


if __name__ == '__main__':
    unittest.main()
