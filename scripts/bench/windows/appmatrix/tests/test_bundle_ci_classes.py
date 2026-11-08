# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Owner CI correction: private media may build and upload only privately."""
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
        B.guard({'CI':'true'},allow_ci=True,bundle_class='private')

    def test_explicit_private_actions_allowed(self):
        B.guard({'GITHUB_ACTIONS':'true'},allow_ci=True,bundle_class='private')

    def test_private_ci_check_command_allowed(self):
        with patch.dict(B.os.environ,{'CI':'true'},clear=True),patch('sys.stdout',new_callable=io.StringIO):
            self.assertEqual(B.main(['check','--class','private','--ci-build']),0)

    def test_private_ci_input_hash_has_no_manifest_contents(self):
        with patch.dict(B.os.environ,{'CI':'true'},clear=True),patch('sys.stdout',new_callable=io.StringIO)as out:
            self.assertEqual(B.main(['input-hash','--class','private']),0)
        self.assertRegex(out.getvalue(),r'\Ainputs=[0-9a-f]{64}\n\Z')

    def test_private_ci_still_rejects_secret(self):
        with self.assertRaises(ValueError): B.guard({'CI':'true','AWS_SECRET_ACCESS_KEY':'fixture'},allow_ci=True,bundle_class='private')

    def test_private_pin_contains_only_iso_digest(self):
        self.assertEqual(B.pin_text('a'*64,'b'*64,'private'),'winapps-private.iso sha256='+'a'*64+'\n')

    def test_same_public_and_private_bucket_refused(self):
        with self.assertRaises(ValueError):P.require_separate_buckets(dict(R2_BUCKET='public',BUNDLE_OTHER_BUCKET='public'))

    def test_same_private_and_public_bucket_refused(self):
        with self.assertRaises(ValueError):P.require_separate_buckets(dict(R2_BUCKET='private',BUNDLE_OTHER_BUCKET='private'))

    def test_private_pin_does_not_reveal_input_hash(self):
        self.assertNotIn('b'*64,B.pin_text('a'*64,'b'*64,'private'))

    def test_private_keys_have_private_namespace(self):
        self.assertTrue(all(x.startswith('winapps-private/')for x in P.keys('a'*64,'private')))

    def test_private_keys_cannot_equal_public_keys(self):
        self.assertFalse(set(P.keys('a'*64,'private'))&set(P.keys('a'*64)))

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
        package=T.package();package['id']='cuda_cudart'
        package['compliance'].update(distribution='private-only',conditions=['private use only'])
        dl=root/'dl';(dl/'cuda_cudart').mkdir(parents=True)
        (dl/'cuda_cudart/LICENSE.txt').write_bytes(b'licence text')
        return B.build_image([package],dl,{},root/'private',{},bundle_class='private')

    def test_public_receipt_cannot_use_private_destination(self):
        with self.assertRaises(ValueError):P.require_class(dict(bundle_class='public',packages=[]),'private')

    def test_public_package_cannot_use_private_destination(self):
        with self.assertRaises(ValueError):P.require_class(dict(bundle_class='private',packages=[dict(id='sevenzip',compliance=dict(distribution='public-ok'))]),'private')

    def test_private_iso_and_manifest_stay_in_private_namespace(self):
        with tempfile.TemporaryDirectory()as tmp:
            root=Path(tmp);receipt=self.image(root,'private');store=MemoryStore()
            keys=P.publish(store,root/'private/winapps-private.iso',receipt,root/'state',bundle_class='private')
            self.assertEqual(set(keys),set(store.objects));self.assertTrue(all(k.startswith('winapps-private/')for k in keys))

    def test_private_republish_does_not_overwrite_manifest(self):
        with tempfile.TemporaryDirectory()as tmp:
            root=Path(tmp);receipt=self.image(root,'private');store=MemoryStore();iso=root/'private/winapps-private.iso'
            keys=P.publish(store,iso,receipt,root/'state',bundle_class='private');first=store.objects[keys[1]][1]
            receipt['packaging_utc']='changed';P.publish(store,iso,receipt,root/'state',bundle_class='private')
            self.assertEqual(first,store.objects[keys[1]][1])


class ClassPlanning(unittest.TestCase):
    def receipt(self,inputs='a'*64,digest='b'*64):return dict(bundle_class='private',inputs=inputs,image=dict(sha256=digest))

    def test_private_manifest_read_is_get_only(self):
        from unittest.mock import Mock
        store=Mock();store.request.return_value=({},B.canonical(self.receipt()))
        C.published_manifest(store,'b'*64,'private')
        store.request.assert_called_once_with('GET',P.keys('b'*64,'private')[1])

    def test_public_manifest_read_is_get_only(self):
        from unittest.mock import Mock
        store=Mock();store.request.return_value=({},b'{}')
        C.published_manifest(store,'b'*64,'public')
        store.request.assert_called_once_with('GET',P.keys('b'*64,'public')[1])

    def test_private_read_forbidden_fails_closed(self):
        from unittest.mock import Mock
        store=Mock();store.request.side_effect=P.StoreError(403)
        with self.assertRaises(P.StoreError):C.published_manifest(store,'b'*64,'private')

    def test_private_read_service_error_fails_closed(self):
        from unittest.mock import Mock
        store=Mock();store.request.side_effect=P.StoreError(500)
        with self.assertRaises(P.StoreError):C.published_manifest(store,'b'*64,'private')

    def test_matching_private_published_manifest_skips(self):
        self.assertFalse(C.plan('a'*64,B.pin_text('b'*64,'c'*64,'private'),bundle_class='private',receipt=self.receipt())['needs_build'])

    def test_changed_private_published_manifest_builds(self):
        self.assertTrue(C.plan('c'*64,B.pin_text('b'*64,'c'*64,'private'),bundle_class='private',receipt=self.receipt())['needs_build'])

    def test_wrong_private_manifest_pin_refused(self):
        with self.assertRaises(ValueError):C.plan('a'*64,B.pin_text('c'*64,'d'*64,'private'),bundle_class='private',receipt=self.receipt())

    def test_wrong_second_private_manifest_pin_refused(self):
        with self.assertRaises(ValueError):C.plan('a'*64,B.pin_text('d'*64,'c'*64,'private'),bundle_class='private',receipt=self.receipt())

    def test_missing_private_manifest_class_refused(self):
        r=self.receipt();r.pop('bundle_class')
        with self.assertRaises(ValueError):C.plan('a'*64,B.pin_text('b'*64,'d'*64,'private'),bundle_class='private',receipt=r)

    def test_wrong_private_manifest_class_refused(self):
        r=self.receipt();r['bundle_class']='public'
        with self.assertRaises(ValueError):C.plan('a'*64,B.pin_text('b'*64,'d'*64,'private'),bundle_class='private',receipt=r)
