"""Structural build-wiring guards supplement the functional colour-selection tests.

GPU-free tests exercise display_color_config; these guards prevent build from
resampling or shadowing that result before constructing its engine and worker.
"""
from pathlib import Path
import re
import unittest

SOURCE = Path(__file__).resolve().parents[2] / 'crates/kf-qemu/src/display.rs'


def build_source():
    text = SOURCE.read_text().split('    pub fn build(', 1)[1]
    text = text.split('    pub fn max_fps(', 1)[0]
    return re.sub(r'//[^\n]*', '', text)


class DisplayBuildWiring(unittest.TestCase):
    def test_build_selects_color_once_without_resampling_or_shadowing(self):
        build = build_source()
        self.assertEqual(build.count('display_color_config('), 1)
        self.assertNotIn('sdr_color_on(', build)
        self.assertNotRegex(build, r'let\s+(?:mut\s+)?sdr_color\s*=')
        self.assertNotRegex(build, r'sdr_color\s*=')

    def test_caps_engine_and_worker_use_the_same_configuration_binding(self):
        build = build_source()
        compact = re.sub(r'\s+', '', build)
        self.assertIn('letDisplayColorConfig{caps,sdr_color,constructor_probe,', compact)
        self.assertIn('}=display_color_config(row,t,&regs)?;', compact)
        self.assertIn('ifconstructor_probe{', compact)
        self.assertIn('sdr_color:sdr_color.then_some((t,classes.window,classes.core))', compact)
        # One binding, field label and its use. A second binding/assignment is a bypass.
        self.assertEqual(len(re.findall(r'\bsdr_color\b', build)), 3)
        self.assertEqual(len(re.findall(r'\bconstructor_probe\b', build)), 2)


if __name__ == '__main__':
    unittest.main()
