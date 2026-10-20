# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Static integration contracts complement the shared wrapper runtime tests."""
import importlib.util
from pathlib import Path
import tomllib
import unittest
spec=importlib.util.spec_from_file_location('wait_audit',Path(__file__).with_name('wait_audit.py'));a=importlib.util.module_from_spec(spec);spec.loader.exec_module(a)
ROOT=Path(__file__).resolve().parents[3]
class QemuWaitTests(unittest.TestCase):
    def code(self):return (ROOT/'crates/kf-qemu/src/device.rs').read_text()
    def test_qemu_device_poll_expressions_are_all_wrapped(self):self.assertEqual(a.unwrapped(self.code()),[])
    def site(self,name):self.assertIn('prof_wait ! ( '+name+' ,',' '.join(t for t,_ in a.tokens(self.code())))
    def test_va_wait_has_a_distinct_site(self):self.site('QemuVaPark')
    def test_drainer_reset_wait_has_a_distinct_site(self):self.site('QemuDrainerResetPark')
    def test_drainer_idle_wait_has_a_distinct_site(self):self.site('QemuDrainerPark')
    def test_profiler_dependency_is_optional_and_feature_is_default_off(self):
        m=tomllib.loads((ROOT/'crates/kf-qemu/Cargo.toml').read_text());self.assertTrue(m['dependencies']['kf-prof']['optional']);self.assertNotIn('prof',m['features'].get('default',[]));self.assertIn('kf-chan/prof',m['features']['prof'])
    def test_loop_registration_happens_before_the_loop_body(self):
        for name,role in [('va_loop',3),('drainer_loop',2),('worker_loop',1)]:
            body=self.code().split('pub fn '+name+'(&self) {',1)[1][:500]
            self.assertIn('bind_current_global('+str(role)+')',body)
    def test_startup_is_compile_gated_and_does_not_gate_boot(self):
        body=self.code().split('pub fn realize(cfg: &Config)',1)[1][:600]
        self.assertRegex(body,r'#\[cfg\(feature = "prof"\)\]\s*let _ = kf_prof::runtime::init_from_env\(\);')
        self.assertIn('cfg.check()?;',body)
if __name__=='__main__':unittest.main()
