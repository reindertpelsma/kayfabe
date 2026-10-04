# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location('bundle_stage', Path(__file__).resolve().parents[1] / 'stage-qga.py')
stage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stage)


class BundleTests(unittest.TestCase):
    def test_external_build_provenance_and_tamper_rejection(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, build = root / 'source', root / 'ci'
            (source / 'build/signing').mkdir(parents=True)
            build.mkdir()
            (source / 'install.ps1').write_text('# fixture source\n')
            (source / 'build/signing/signtool.exe').write_bytes(b'fixture signing tool')
            artifacts = []
            for name in ('gsptrace.sys', 'gsptrace.exe', 'windows_api_test.exe'):
                contents = ('fixture ' + name).encode()
                (build / name).write_bytes(contents)
                artifacts.append(dict(name=name, bytes=len(contents), sha256=hashlib.sha256(contents).hexdigest()))
            metadata = dict(completed=True, signing_performed=False, source_revision='2' * 40, artifacts=artifacts)
            metadata_path = build / 'build-info.json'
            metadata_path.write_text(json.dumps(metadata))
            output = root / 'bundle.zip'

            def git(arguments, **kwargs):
                return '1' * 40 if 'rev-parse' in arguments else b'install.ps1\0'

            with patch.object(stage, 'HERE', source), patch.object(stage.subprocess, 'check_output', side_effect=git), contextlib.redirect_stdout(io.StringIO()):
                stage.bundle(output, build)
                with zipfile.ZipFile(output) as archive:
                    manifest = json.loads(archive.read('stage-manifest.json'))
                    self.assertEqual(manifest['source_revision'], '1' * 40)
                    self.assertEqual(manifest['build_source_revision'], '2' * 40)
                    self.assertEqual(archive.read('build/gsptrace.sys'), (build / 'gsptrace.sys').read_bytes())
                    self.assertIn('build/build-info.json', archive.namelist())
                    self.assertIn('build/signing/signtool.exe', archive.namelist())
                (build / 'gsptrace.sys').write_bytes(b'changed')
                with self.assertRaisesRegex(RuntimeError, 'provenance mismatch'):
                    stage.bundle(output, build)
                metadata_path.unlink()
                with self.assertRaisesRegex(RuntimeError, 'requires build-info.json'):
                    stage.bundle(output, build)


if __name__ == '__main__':
    unittest.main()
