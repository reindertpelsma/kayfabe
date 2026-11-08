# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import helpers as H
import official_runtime as O
import unittest
from pathlib import Path
import tempfile
from unittest.mock import patch
import package_bundle as B
import bundle_iso as I
import hashlib
import copy
import bundle_repro as R
import warnings
import io
import os
import json

class Official(unittest.TestCase):
    def test_generated_catalogue_matches_manifest_exactly(self):
        import gen_official as G
        self.assertEqual(B.load(B.HERE/'official_fetch.json'),G.catalogue(B.load(B.HERE/'manifest.json')))

    def test_generated_wheel_lock_matches_pinned_direct_artifacts(self):
        import gen_official as G
        data=G.catalogue(B.load(B.HERE/'manifest.json'))
        lock=(B.HERE/'official_requirements.txt').read_text()
        self.assertEqual(lock,G.lock(data))
        self.assertTrue(all(' @ https://'in line and ' --hash=sha256:'in line for line in lock.splitlines()if not line.startswith('#')))

    def test_j2_driver_plan_is_guest_only_and_exact(self):
        data=B.load(B.HERE/'official_fetch.json');plan=O.driver_plan(data,'580.88')
        self.assertEqual(plan['version'],'580.88');self.assertEqual(plan['stage_package'],'nvidia_driver')
        self.assertEqual(plan['installer_argv'][0],r'C:\Windows\System32\pnputil.exe')
        self.assertIn('Display.Driver',plan['installer_argv'][2]);self.assertNotIn('{dest}',' '.join(plan['installer_argv']))

    def test_inventory_has_exact_pins(self):
        d=B.load(B.HERE/'official_fetch.json');O.validate(d)
        self.assertEqual(len(d['entries']),33)

    def test_inventory_contains_llama_cudart(self):
        d=B.load(B.HERE/'official_fetch.json')
        self.assertTrue(any(e['name'].startswith('cudart-llama')for e in d['entries']))

    def test_hash_failure_is_media_drift(self):
        with tempfile.TemporaryDirectory()as t:
            p=Path(t)/'fetch.py';p.write_bytes(b'changed')
            with self.assertRaises(O.MediaDrift):O.check_fetcher(p,{'sha256':'a'*64})

    def test_missing_fetcher_is_media_drift(self):
        with self.assertRaises(O.MediaDrift):O.check_fetcher(Path('/missing/official-fetcher'),{'sha256':'a'*64})

    def test_http_failure_is_media_drift(self):
        with patch.object(O.subprocess,'run',return_value=type('R',(),{'returncode':1})()):
            with self.assertRaises(O.MediaDrift):O.download({'id':'fixture','official_url':'https://example.invalid','size':1,'sha256':'a'*64},'out','fetch.py')

    def test_upstream_hash_mismatch_is_media_drift(self):
        with patch.object(O.subprocess,'run',return_value=type('R',(),{'returncode':3})()):
            with self.assertRaises(O.MediaDrift):O.download({'id':'fixture','official_url':'https://example.invalid','size':1,'sha256':'a'*64},'out','fetch.py')

    def nonzero_with_complete_bytes(self,rc):
        d,e=self.fixture()
        with tempfile.TemporaryDirectory()as tmp,patch.object(O.subprocess,'run',return_value=type('R',(),{'returncode':rc})()):
            p=Path(tmp)/'original';p.write_bytes(b'x')
            with self.assertRaises(O.MediaDrift):O.download(e,p,'fetch.py')

    def test_network_error_after_write_is_not_success(self):self.nonzero_with_complete_bytes(1)
    def test_hash_error_after_write_is_not_success(self):self.nonzero_with_complete_bytes(3)

    def environment(self,secret):
        d,e=self.fixture()
        with tempfile.TemporaryDirectory()as tmp,patch.dict(os.environ,{secret:'fixture-value'},clear=True),patch.object(O.subprocess,'run',return_value=type('R',(),{'returncode':0})())as run,patch('sys.stdout',new_callable=io.StringIO):
            p=Path(tmp)/'original';p.write_bytes(b'x');O.download(e,p,'fetch.py')
            self.assertNotIn(secret,run.call_args.kwargs['env']);self.assertFalse(run.call_args.kwargs['shell'])
            self.assertNotIn('--worker-auth',run.call_args.args[0])

    def test_run_token_not_forwarded_to_official_origin(self):self.environment('KF_RUN_TOKEN')
    def test_upload_secret_not_forwarded_to_official_fetcher(self):self.environment('AWS_SECRET_ACCESS_KEY')

    def test_required_groups_resolve_to_exact_entries(self):
        d=B.load(B.HERE/'official_fetch.json');ids=O.select(d,[dict(requires={'official_fetch':['cuda_cudart']})],'580.88')
        self.assertIn(d['driver_versions']['580.88'],ids)
        self.assertTrue(set(d['packages']['cuda_cudart'])<=set(ids))

    def test_public_only_selection_needs_no_transient_disk(self):
        self.assertEqual(O.select(B.load(B.HERE/'official_fetch.json'),[dict(pkgs=[])],None),[])

    def test_unpinned_selected_driver_refused(self):
        with self.assertRaises(O.MediaDrift):O.select(B.load(B.HERE/'official_fetch.json'),[],'999.99')

    def test_missing_selected_driver_refused_for_official_apps(self):
        with self.assertRaises(O.MediaDrift):O.select(B.load(B.HERE/'official_fetch.json'),[dict(requires={'official_fetch':['cuda_cudart']})],None)

    def test_empty_selected_driver_is_not_a_fallback(self):
        with self.assertRaises(O.MediaDrift):O.select(B.load(B.HERE/'official_fetch.json'),[dict(requires={'official_fetch':['pywheels']})],'')

    def bad_origin(self,host):
        d,e=self.fixture();e['official_url']='https://'+host+'/fixed.zip';d['entries']=[e]
        with self.assertRaises(O.MediaDrift):O.validate(d)

    def test_nonofficial_host_refused(self):self.bad_origin('vendor.invalid')
    def test_nvidia_name_in_unrelated_host_refused(self):self.bad_origin('nvidia.com.vendor.invalid')

    def bad_destination(self,dest):
        d,e=self.fixture();e['guest_destination']=dest;d['entries']=[e]
        with self.assertRaises(O.MediaDrift):O.validate(d)

    def test_guest_system_directory_is_not_app_destination(self):self.bad_destination('C:\\Windows\\System32')
    def test_guest_destination_cannot_traverse(self):self.bad_destination('C:\\kfapps\\..\\Windows')

    def test_driver_catalogue_key_matches_exact_version(self):
        d=B.load(B.HERE/'official_fetch.json');d['driver_versions']['999.99']=d['driver_versions'].pop('580.88')
        with self.assertRaises(O.MediaDrift):O.validate(d)

    def test_driver_catalogue_cannot_name_cuda_runtime(self):
        d=B.load(B.HERE/'official_fetch.json');d['driver_versions']['580.88']=d['packages']['cuda_cudart'][0]
        with self.assertRaises(O.MediaDrift):O.validate(d)

    def fixture(self):
        data=B.load(B.HERE/'official_fetch.json');e=copy.deepcopy(next(e for e in data['entries']if e['package']=='cuda_cudart'))
        e.update(size=1,sha256=hashlib.sha256(b'x').hexdigest())
        data['entries']=[e];data['packages']={e['package']:[e['id']]};data['driver_versions']={}
        return data,e

    def transient(self,data,entry):
        return O.transient(data,[entry['id']],'fetch.py')

    def test_transient_contains_original_archive_and_fixed_label(self):
        d,e=self.fixture()
        def fake(entry,destination,fetcher):Path(destination).write_bytes(b'x')
        with patch.object(O,'check_fetcher'),patch.object(O,'download',side_effect=fake):
            with self.transient(d,e)as iso:
                with I.Reader(iso)as image:
                    self.assertEqual(image.label,'KFAPPSP');self.assertEqual(b''.join(image.chunks('pkg/'+e['package']+'/'+e['name'])),b'x')
                self.assertTrue(iso.exists())
            self.assertFalse(iso.parent.exists())

    def test_transient_cleanup_after_caller_error(self):
        d,e=self.fixture()
        with patch.object(O,'check_fetcher'),patch.object(O,'download',side_effect=lambda e,p,f:Path(p).write_bytes(b'x')):
            with self.assertRaisesRegex(RuntimeError,'fixture'):
                with self.transient(d,e)as iso:raise RuntimeError('fixture')
            self.assertFalse(iso.parent.exists())

    def test_transient_manifest_matches_its_volume_label(self):
        d,e=self.fixture()
        with patch.object(O,'check_fetcher'),patch.object(O,'download',side_effect=lambda e,p,f:Path(p).write_bytes(b'x')):
            with self.transient(d,e)as iso:
                manifest=B.load(iso.parent/'official-run.manifest.json')
                with I.Reader(iso)as image:self.assertEqual(image.label,manifest['image']['label'])
                self.assertEqual(manifest['image']['label'],'KFAPPSP')

    def test_transient_rejects_download_changed_during_assembly(self):
        d,e=self.fixture()
        with patch.object(O,'check_fetcher'),patch.object(O,'download',side_effect=lambda e,p,f:Path(p).write_bytes(b'y')):
            with self.assertRaises(O.MediaDrift):
                with self.transient(d,e):pass

    def test_transient_rejects_truncated_original(self):
        d,e=self.fixture()
        with patch.object(O,'check_fetcher'),patch.object(O,'download',side_effect=lambda e,p,f:Path(p).write_bytes(b'')):
            with self.assertRaises(O.MediaDrift):
                with self.transient(d,e):pass

    def test_transient_bytes_are_deterministic(self):
        d,e=self.fixture();receipts=[]
        with patch.object(O,'check_fetcher'),patch.object(O,'download',side_effect=lambda e,p,f:Path(p).write_bytes(b'x')):
            for _ in range(2):
                with self.transient(d,e)as iso:receipts.append(R.read_candidate(iso,B.load(iso.parent/'official-run.manifest.json')))
        result=R.compare(receipts[0],receipts[1]);self.assertEqual(result['level1'],'equal')
        if result['level2']!='equal':warnings.warn('LEVEL2_CONTAINER_DIFF '+json.dumps(result['container_diff']))
