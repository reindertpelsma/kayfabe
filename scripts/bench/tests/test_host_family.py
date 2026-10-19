#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Tier1 health checks must not reject healthy non-Ampere NVIDIA boards by name."""
import os
from pathlib import Path
import subprocess
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'host_preflight.sh'


class HostFamilyTests(unittest.TestCase):
    def check(self, name):
        source = SCRIPT.read_text()
        block = source[source.index('# 2.'):source.index('# 3.')]
        fixture = '''FAIL=0; WRONG=0
ok() { :; }
bad() { FAIL=1; }
wrong() { WRONG=1; }
nvidia-smi() {
 case "$*" in *driver_version*) echo 595.91.07 ;; *) printf '%s\\n' "$GPU_NAME" ;; esac
}
'''
        r = subprocess.run(['bash', '-c', fixture + block + '\necho CHECK=$FAIL,$WRONG'],
                           env=dict(os.environ, GPU_NAME=name), capture_output=True,
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


if __name__ == '__main__':
    unittest.main()
