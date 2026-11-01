#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Tier1 health checks must not reject healthy non-Ampere NVIDIA boards by name."""
import os
from pathlib import Path
import subprocess
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'host_preflight.sh'


class HostFamilyTests(unittest.TestCase):
    def check(self, name, driver='595.91.07', name_rc=0, driver_rc=0):
        source = SCRIPT.read_text()
        block = source[source.index('# 2.'):source.index('# 3.')]
        fixture = '''FAIL=0; WRONG=0
ok() { :; }
bad() { FAIL=1; }
wrong() { WRONG=1; }
nvidia-smi() {
 case "$*" in
  *driver_version*) printf '%s\\n' "$GPU_DRIVER"; return "$DRIVER_RC" ;;
  *) printf '%s\\n' "$GPU_NAME"; return "$NAME_RC" ;;
 esac
}
'''
        r = subprocess.run(['bash', '-c', fixture + block + '\necho CHECK=$FAIL,$WRONG'],
                           env=dict(os.environ, GPU_NAME=name, GPU_DRIVER=driver,
                                    NAME_RC=str(name_rc), DRIVER_RC=str(driver_rc)), capture_output=True,
                           text=True, timeout=5)
        return r.stdout

    def test_every_target_family_is_eligible_for_host_derived_gates(self):
        for name in ('NVIDIA GeForce RTX 2060 SUPER', 'NVIDIA GeForce RTX 3070',
                     'NVIDIA GeForce RTX 4090', 'NVIDIA GeForce RTX 5060'):
            with self.subTest(name=name):
                self.assertIn('CHECK=0,0', self.check(name))

    def test_healthy_datacenter_names_are_eligible_without_a_geforce_whitelist(self):
        for name in ('NVIDIA A100-SXM4-40GB', 'NVIDIA H100 PCIe'):
            with self.subTest(name=name):
                self.assertIn('CHECK=0,0', self.check(name))

    def test_a_driver_returning_no_gpu_still_fails_health(self):
        self.assertIn('CHECK=1,0', self.check(''))

    def test_no_devices_error_is_not_a_gpu_even_with_a_numeric_driver(self):
        self.assertIn('CHECK=1,0', self.check('No devices were found'))

    def test_no_devices_error_with_failed_queries_is_not_sound(self):
        self.assertIn('CHECK=1,0', self.check('No devices were found',
                                          driver='No devices were found',
                                          name_rc=6, driver_rc=6))

    def test_failed_name_query_cannot_report_a_cached_gpu_as_healthy(self):
        self.assertIn('CHECK=1,0', self.check('NVIDIA GeForce RTX 5090', name_rc=6))

    def test_transient_name_query_failure_cannot_report_a_healthy_gpu(self):
        self.assertIn('CHECK=1,0', self.check('NVIDIA H100 PCIe', name_rc=1))

    def test_failed_driver_query_cannot_report_a_cached_version_as_healthy(self):
        self.assertIn('CHECK=1,0', self.check('NVIDIA GeForce RTX 5090', driver_rc=6))

    def test_transient_driver_query_failure_cannot_report_a_healthy_gpu(self):
        self.assertIn('CHECK=1,0', self.check('NVIDIA H100 PCIe', driver_rc=1))

    def test_empty_driver_query_is_not_a_driver_identity(self):
        self.assertIn('CHECK=1,0', self.check('NVIDIA GeForce RTX 5090', driver=''))

    def test_error_text_is_not_a_driver_version_even_with_zero_exit_status(self):
        self.assertIn('CHECK=1,0', self.check('NVIDIA GeForce RTX 5090',
                                          driver='No devices were found'))

    def test_malformed_numeric_driver_version_is_not_accepted(self):
        self.assertIn('CHECK=1,0', self.check('NVIDIA GeForce RTX 5090', driver='595.bad'))

    def test_multiline_successful_queries_keep_the_first_gpu_identity(self):
        self.assertIn('CHECK=0,0', self.check('NVIDIA H100 PCIe\nNVIDIA H100 PCIe',
                                          driver='595.91.07\n595.91.07'))


if __name__ == '__main__':
    unittest.main()
