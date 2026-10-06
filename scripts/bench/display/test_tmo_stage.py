#!/usr/bin/env python3
"""A compositor/atomic success or transient LUT binding must not pass the oracle."""
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

from PIL import Image
import check_tmo_stage


class StrictTmoGate(unittest.TestCase):
    def fixture(self, root, methods, request=True, missing=False):
        (root / 'manifest.json').write_text(json.dumps(dict(
            required_tmo=True, scene_grayscale=True, source_revision='a' * 40,
            qemu_sha256='b' * 64, harness_sha256='c' * 64)))
        table = root / 'classes.tsv'
        table.write_text('V\tNVC67E_SET_CONTEXT_DMA_TMO_LUT\t1320\n'
                         'V\tNVC67E_SET_TMO_CONTROL\t1280\n'
                         'V\tNVC67E_UPDATE\t512\n'
                         'F\tNVC67E_SET_TMO_CONTROL_SIZE\t18\t8\n')
        ready = 'WL_SCENE_READY\nWL_SCENE_RENDERER NVIDIA fixture\n'
        (root / 'sway-before.log').write_text(ready + 'TMO_TEST armed\n')
        warm = ready + ('TMO_TEST MISSING_TMO_LUT\n' if missing else '')
        if not missing:
            warm += ''.join(f'COLOR_LUT object=30 name=TMO_LUT index={i} rgb=0,0,0\n'
                            for i in range(1024))
        (root / 'sway-warm.log').write_text(warm)
        atomic = ('TMO_TEST REQUEST plane=30 prop=40 blob=50 flags=0 test_only=0 rc=0 errno=0\n'
                  if request else '')
        (root / 'sway-restored.log').write_text(ready + warm + atomic)
        trace = ''.join(f'METHOD chn=1 kind=Window method={m:#x} data={v:#x} remaining=100\n'
                        for m, v in methods)
        (root / 'qemu.log').write_text(trace)
        (root / 'sway-warm-trace.log').write_text(trace)
        for name, value in [('sway-before', 128), ('sway-warm', 0), ('sway-restored', 128)]:
            Image.new('RGB', (1920, 1080), (value,) * 3).save(root / (name + '.ppm'))
        return table

    def grade(self, methods, **kwargs):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            table = self.fixture(root, methods, **kwargs)
            out = root / 'result.json'
            with patch.object(sys, 'argv', ['check', str(root), str(table), str(out)]), \
                 contextlib.redirect_stdout(io.StringIO()):
                try:
                    check_tmo_stage.main()
                    rc = 0
                except SystemExit as error:
                    rc = error.code
            return rc, json.loads(out.read_text())

    def test_missing_property_cannot_pass_even_with_black_pixels(self):
        rc, result = self.grade([(0x200, 0)], missing=True, request=False)
        self.assertEqual(rc, 1)
        self.assertIn('tmo_property_available', result['failure_reasons'])

    def test_accepted_atomic_and_black_pixels_without_binding_fail(self):
        rc, result = self.grade([(0x200, 0)])
        self.assertEqual(rc, 1)
        self.assertIn('nonzero_tmo_method_binding', result['failure_reasons'])

    def test_binding_without_update_is_not_armed(self):
        rc, result = self.grade([(0x528, 7), (0x500, 1029 << 8)])
        self.assertEqual(rc, 1)
        self.assertIn('nonzero_tmo_binding_armed_at_capture', result['failure_reasons'])

    def test_disabled_binding_at_capture_fails(self):
        bound = [(0x528, 7), (0x500, 1029 << 8), (0x200, 0)]
        rc, result = self.grade(bound + [(0x528, 0), (0x200, 0)])
        self.assertEqual(rc, 1)
        self.assertIn('nonzero_tmo_binding_armed_at_capture', result['failure_reasons'])

    def test_complete_synthetic_witness_checks_the_fixture(self):
        # This only tests the grader; it is not hardware evidence.
        rc, result = self.grade([(0x528, 7), (0x500, 1029 << 8), (0x200, 0)])
        self.assertEqual(rc, 0)
        self.assertTrue(result['passed'])


if __name__ == '__main__':
    unittest.main()
