# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Owner policy: public-only permanent publication and exact manifest binding."""
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import helpers as H
import package_bundle as B
import bundle_ci as C
import publish_bundle as P
import test_bundle_compliance as T
from test_bundle_publish import MemoryStore


class PrivateCI(unittest.TestCase):
    def test_explicit_private_ci_allowed(self):
        with self.assertRaises(ValueError):B.guard({'CI':'true'},allow_ci=True,bundle_class='private')

    def test_explicit_private_actions_allowed(self):
        with self.assertRaises(ValueError):B.guard({'GITHUB_ACTIONS':'true'},allow_ci=True,bundle_class='private')

    def test_retired_private_ci_check_command_refused(self):
        with patch.dict(B.os.environ,{'CI':'true'},clear=True),patch('sys.stdout',new_callable=io.StringIO):
            self.assertEqual(B.main(['check','--class','private','--ci-build']),1)

    def test_private_ci_input_hash_has_no_manifest_contents(self):
        with patch.dict(B.os.environ,{'CI':'true'},clear=True),patch('sys.stdout',new_callable=io.StringIO)as out:
            self.assertEqual(B.main(['input-hash','--class','private']),1)
        self.assertEqual(out.getvalue(),'')

    def test_private_ci_still_rejects_secret(self):
        with self.assertRaises(ValueError): B.guard({'CI':'true','AWS_SECRET_ACCESS_KEY':'fixture'},allow_ci=True,bundle_class='private')

    def test_private_pin_contains_only_iso_digest(self):
        with self.assertRaises(ValueError):B.pin_text('a'*64,'b'*64,'private')

    def test_private_pin_does_not_reveal_input_hash(self):
        with self.assertRaises(ValueError):B.pin_text('a'*64,'b'*64,'transient-official')

    def test_runtime_has_no_release_object_keys(self):
        with self.assertRaises(ValueError):P.keys('a'*64,'private')

    def test_transient_has_no_release_object_keys(self):
        with self.assertRaises(ValueError):P.keys('a'*64,'transient-official')

    def test_private_logs_hide_filename(self):
        pin=dict(name='PRIVATE-CONTENTS.exe',size=7,sha256='a'*64,url='https://example.invalid/file')
        with tempfile.TemporaryDirectory()as tmp,patch.object(B,'verify_file'),patch('sys.stdout',new_callable=io.StringIO)as out:
            path=Path(tmp)/'cached';path.write_bytes(b'payload')
            B.fetch_file('cuda_cudart',pin,path,private=True)
        self.assertNotIn('PRIVATE-CONTENTS',out.getvalue());self.assertIn('cuda_cudart',out.getvalue())

    def test_private_logs_include_only_pinned_metadata(self):
        pin=dict(name='secret.zip',size=3,sha256='a'*64,url='https://example.invalid/file')
        with tempfile.TemporaryDirectory()as tmp,patch.object(B,'verify_file'),patch('sys.stdout',new_callable=io.StringIO)as out:
            path=Path(tmp)/'cached';path.write_bytes(b'xxx');B.fetch_file('pywheels',pin,path,private=True)
        self.assertEqual(out.getvalue(),'CACHE_OK package=pywheels bytes=3 sha256='+'a'*64+'\n')

    def test_actions_without_explicit_flag_refused_public(self):
        with self.assertRaises(ValueError):B.guard({'GITHUB_ACTIONS':'true'},bundle_class='public')

    def test_actions_cli_without_explicit_flag_refused_public(self):
        with patch.dict(B.os.environ,{'GITHUB_ACTIONS':'true'},clear=True),patch('sys.stderr',new_callable=io.StringIO):
            self.assertEqual(B.main(['check']),1)

    def test_actions_without_explicit_flag_refused_private(self):
        with self.assertRaises(ValueError):B.guard({'GITHUB_ACTIONS':'true'},bundle_class='private')

    def test_private_error_does_not_log_vendor_text(self):
        with patch.dict(B.os.environ,{},clear=True),patch.object(B,'load',side_effect=ValueError('PRIVATE-CONTENTS')),patch('sys.stderr',new_callable=io.StringIO)as err:
            self.assertEqual(B.main(['check','--class','private']),1)
        self.assertNotIn('PRIVATE-CONTENTS',err.getvalue())

    def test_private_error_does_not_log_url(self):
        with patch.dict(B.os.environ,{},clear=True),patch.object(B,'load',side_effect=ValueError('https://private.invalid/signed')),patch('sys.stderr',new_callable=io.StringIO)as err:
            self.assertEqual(B.main(['check','--class','private']),1)
        self.assertNotIn('https://',err.getvalue())


class PrivatePublishing(unittest.TestCase):
    def image(self, root, cls):
        return dict(bundle_class='transient-official',packages=[],inputs='a'*64,image=dict(sha256='b'*64,size=1))

    def test_public_receipt_cannot_use_private_destination(self):
        with self.assertRaises(ValueError):P.require_class(dict(bundle_class='public',packages=[]),'private')

    def test_public_package_cannot_use_private_destination(self):
        with self.assertRaises(ValueError):P.require_class(dict(bundle_class='private',packages=[dict(id='sevenzip',compliance=dict(distribution='public-ok'))]),'private')

    def test_public_receipt_cannot_use_transient_destination(self):
        with self.assertRaises(ValueError):P.require_class(dict(bundle_class='public',packages=[]),'transient-official')

    def test_runtime_iso_never_published(self):
        with tempfile.TemporaryDirectory()as tmp:
            root=Path(tmp);receipt=self.image(root,'private');store=MemoryStore()
            with self.assertRaises(ValueError):P.publish(store,root/'private/winapps-private.iso',receipt,root/'state',bundle_class='private')
            self.assertFalse(store.calls)

    def test_runtime_manifest_never_published(self):
        with tempfile.TemporaryDirectory()as tmp:
            root=Path(tmp);receipt=self.image(root,'private');store=MemoryStore()
            with self.assertRaises(ValueError):P.publish(store,root/'private/winapps-private.iso',receipt,root/'state',bundle_class='private')
            self.assertFalse(store.objects)


class ClassPlanning(unittest.TestCase):
    def receipt(self,inputs='a'*64,digest='b'*64):return dict(bundle_class='public',inputs=inputs,image=dict(sha256=digest))

    def test_matching_public_published_manifest_skips(self):
        self.assertFalse(C.plan('a'*64,B.pin_text('b'*64,'c'*64),bundle_class='public',receipt=self.receipt())['needs_build'])

    def test_changed_public_published_manifest_builds(self):
        self.assertTrue(C.plan('c'*64,B.pin_text('b'*64,'c'*64),bundle_class='public',receipt=self.receipt())['needs_build'])

    def test_wrong_public_manifest_pin_refused(self):
        with self.assertRaises(ValueError):C.plan('a'*64,B.pin_text('c'*64,'d'*64),bundle_class='public',receipt=self.receipt())

    def test_wrong_second_public_manifest_pin_refused(self):
        with self.assertRaises(ValueError):C.plan('a'*64,B.pin_text('d'*64,'c'*64),bundle_class='public',receipt=self.receipt())

    def test_missing_public_manifest_class_refused(self):
        r=self.receipt();r.pop('bundle_class')
        with self.assertRaises(ValueError):C.plan('a'*64,B.pin_text('b'*64,'d'*64),bundle_class='public',receipt=r)

    def test_wrong_public_manifest_class_refused(self):
        r=self.receipt();r['bundle_class']='transient-official'
        with self.assertRaises(ValueError):C.plan('a'*64,B.pin_text('b'*64,'d'*64),bundle_class='public',receipt=r)
