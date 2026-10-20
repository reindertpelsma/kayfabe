# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import unittest
import zero_cost
class ZeroCostTests(unittest.TestCase):
 def test_runtime_text_symbols_are_detected(self):self.assertEqual(zero_cost.profiler_symbols('0001 T kf_prof::runtime::site::habc\n'),['kf_prof::runtime::site::habc'])
 def test_runtime_data_and_mangled_symbols_are_detected(self):
  self.assertEqual(len(zero_cost.profiler_symbols('0000 B kf_prof::runtime::ON\n0001 T _RNvC123kf_prof4site\n')),2)
 def test_legacy_census_is_not_mistaken_for_the_new_crate(self):self.assertEqual(zero_cost.profiler_symbols('0001 T kf_qemu::prof::on\n'),[])
 def test_symbol_identity_ignores_locations_and_rust_hashes(self):self.assertEqual(zero_cost.public_symbols('0001 T kf_qemu::device::read::h0123456789abcdef\n'),zero_cost.public_symbols('99ff T kf_qemu::device::read::habcdef0123456789\n'))
if __name__=='__main__':unittest.main()
