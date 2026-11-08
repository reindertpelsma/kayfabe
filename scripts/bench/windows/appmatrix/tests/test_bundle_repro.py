# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Level 1 gates payload; level 2 diagnoses container differences only."""
import copy
import hashlib
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import urllib.error

import helpers as H
import package_bundle as B
import bundle_iso as I
import bundle_repro as R


def receipt(files):
    return dict(files={p:dict(sha256=hashlib.sha256(b).hexdigest(),size=len(b))for p,b in files.items()},
                image=dict(sha256='a'*64,size=0),toolchain={})


class Inventory(unittest.TestCase):
    def test_same_files_and_hashes_pass(self):
        a=receipt({'a':b'a'});self.assertEqual(R.compare(a,copy.deepcopy(a))['level1'],'equal')

    def test_changed_vendor_bytes_fail(self):
        self.assertEqual(R.compare(receipt({'pkg/a':b'a'}),receipt({'pkg/a':b'b'}))['level1'],'different')

    def test_changed_program_bytes_fail_by_default(self):
        self.assertEqual(R.compare(receipt({'tools/kf_dxprobe.exe':b'a'}),receipt({'tools/kf_dxprobe.exe':b'b'}))['level1'],'different')

    def test_added_file_fails(self):
        self.assertEqual(R.compare(receipt({'a':b'a'}),receipt({'a':b'a','b':b'b'}))['level1'],'different')

    def test_added_licence_file_fails(self):
        self.assertEqual(R.compare(receipt({'a':b'a'}),receipt({'a':b'a','LICENSES/extra':b'b'}))['level1'],'different')

    def test_missing_file_fails(self):
        self.assertEqual(R.compare(receipt({'a':b'a','b':b'b'}),receipt({'a':b'a'}))['level1'],'different')

    def test_missing_test_program_fails(self):
        self.assertEqual(R.compare(receipt({'a':b'a','tools/cup2.exe':b'b'}),receipt({'a':b'a'}))['level1'],'different')

    def test_container_hash_change_is_warning(self):
        a=receipt({'a':b'a'});b=copy.deepcopy(a);b['image']['sha256']='b'*64
        result=R.compare(a,b);self.assertEqual(result['level1'],'equal');self.assertEqual(result['level2'],'different')

    def test_container_size_change_is_warning(self):
        a=receipt({'a':b'a'});b=copy.deepcopy(a);b['image']['size']=2048
        self.assertEqual(R.compare(a,b)['level1'],'equal')

    def test_external_audit_date_not_payload(self):
        a=receipt({'a':b'a'});b=copy.deepcopy(a);b['packaging_utc']='2099-01-01'
        self.assertEqual(R.compare(a,b)['level1'],'equal')

    def test_identical_container_has_no_warning(self):
        a=receipt({'a':b'a'});result=R.compare(a,copy.deepcopy(a))
        self.assertEqual(result['level2'],'equal')
        with patch('sys.stdout',new_callable=io.StringIO)as out:R.report(result)
        self.assertNotIn('WARNING',out.getvalue())

    def test_external_python_version_does_not_create_warning(self):
        a=receipt({'a':b'a'});b=copy.deepcopy(a);b['python_version']='different host'
        self.assertEqual(R.compare(a,b)['level2'],'equal')

    def test_order_does_not_change_level1_digest(self):
        self.assertEqual(R.inventory_hash(receipt({'a':b'a','b':b'b'})),R.inventory_hash(receipt({'b':b'b','a':b'a'})))


class Exceptions(unittest.TestCase):
    def qualified(self,data):
        r=receipt({'tools/kf_dxprobe.exe':data})
        policy=dict(mode='inventory-only',reason='Cross-host fixture',functional_checks=['dxgi_adapters'])
        r['toolchain']['programs']={'kf_dxprobe.exe':policy}
        r['files']['tools/kf_dxprobe.exe']['reproducibility']=policy
        return r

    def test_explicit_program_inventory_exception(self):
        result=R.compare(self.qualified(b'a'),self.qualified(b'different-size'))
        self.assertEqual(result['level1'],'equal');self.assertIn('tools/kf_dxprobe.exe',result['inventory_only'])

    def test_one_sided_exception_cannot_hide_program_difference(self):
        a=receipt({'tools/kf_dxprobe.exe':b'a'})
        self.assertEqual(R.compare(a,self.qualified(b'b'))['level1'],'different')

    def test_vendor_cannot_be_inventory_only(self):
        r=receipt({'pkg/vendor.exe':b'a'});r['files']['pkg/vendor.exe']['reproducibility']=dict(mode='inventory-only',reason='x',functional_checks=['dxgi_adapters'])
        with self.assertRaises(ValueError):R.inventory(r)

    def test_license_text_cannot_be_inventory_only(self):
        r=receipt({'LICENSES/vendor.txt':b'a'});r['files']['LICENSES/vendor.txt']['reproducibility']=dict(mode='inventory-only',reason='x',functional_checks=['dxgi_adapters'])
        with self.assertRaises(ValueError):R.inventory(r)

    def test_exception_needs_reason(self):
        r=self.qualified(b'a');r['toolchain']['programs']['kf_dxprobe.exe']['reason']=''
        with self.assertRaises(ValueError):R.inventory(r)

    def test_exception_needs_functional_check(self):
        r=self.qualified(b'a');r['toolchain']['programs']['kf_dxprobe.exe']['functional_checks']=[]
        with self.assertRaises(ValueError):R.inventory(r)


class Containers(unittest.TestCase):
    def image(self,root):
        iso=root/'test.iso';I.write_image({'a.txt':b'unchanged'},iso)
        with I.Reader(iso)as reader:
            r=dict(files=reader.records(),image=dict(sha256=B.sha256(iso),size=iso.stat().st_size),toolchain={},container_metadata=reader.metadata())
        return iso,r

    def altered(self,offset):
        with tempfile.TemporaryDirectory()as tmp:
            iso,a=self.image(Path(tmp));data=bytearray(iso.read_bytes());data[offset]^=1;iso.write_bytes(data)
            b=copy.deepcopy(a);b['image']['sha256']=B.sha256(iso)
            b=R.read_candidate(iso,b)
            result=R.compare(a,b);self.assertEqual(result['level1'],'equal');self.assertEqual(result['level2'],'different');self.assertTrue(result['container_diff'])
            return result

    def test_publisher_metadata_difference_is_warning(self):self.altered(16*2048+318)
    def test_date_metadata_difference_is_warning(self):self.altered(16*2048+813)
    def test_system_area_difference_is_warning(self):self.altered(100)

    def test_downloaded_object_hash_is_still_enforced(self):
        with tempfile.TemporaryDirectory()as tmp:
            iso,r=self.image(Path(tmp));data=bytearray(iso.read_bytes());data[100]^=1;iso.write_bytes(data)
            with self.assertRaisesRegex(ValueError,'INTEGRITY'):R.read_candidate(iso,r)

    def test_downloaded_object_size_is_still_enforced(self):
        with tempfile.TemporaryDirectory()as tmp:
            iso,r=self.image(Path(tmp));r['image']['size']+=2048
            with self.assertRaisesRegex(ValueError,'INTEGRITY'):R.read_candidate(iso,r)

    def test_own_manifest_cannot_claim_extra_vendor_file(self):
        self.extra_own_file('pkg/vendor.zip')

    def test_own_manifest_cannot_claim_extra_program(self):
        self.extra_own_file('tools/cup2.exe')

    def extra_own_file(self,name):
        with tempfile.TemporaryDirectory()as tmp:
            iso,r=self.image(Path(tmp));r['files'][name]=dict(sha256='a'*64,size=1)
            with self.assertRaisesRegex(ValueError,'file list'):R.read_candidate(iso,r)

    def test_own_payload_hash_is_still_enforced(self):
        with tempfile.TemporaryDirectory()as tmp:
            iso,r=self.image(Path(tmp))
            with I.Reader(iso)as image:offset=image.files['a.txt'][0]
            with iso.open('r+b')as image:image.seek(offset);image.write(b'X')
            r['image']['sha256']=B.sha256(iso)
            with self.assertRaisesRegex(ValueError,'candidate file'):R.read_candidate(iso,r)


class BuiltPrograms(unittest.TestCase):
    def build(self,root,tool_data,inventory_only):
        import test_package_bundle as T
        packages=T.manifest()['packages'];dl=root/'downloads'
        (dl/'sevenzip').mkdir(parents=True);(dl/'sevenzip/7zr.exe').write_bytes(b'installer stays intact')
        (dl/'_compliance/sevenzip').mkdir(parents=True);(dl/'_compliance/sevenzip/LICENSE.txt').write_bytes(b'fixture licence')
        tool=root/'probe.exe';tool.write_bytes(tool_data)
        tc={'programs':{'kf_dxprobe.exe':dict(mode='inventory-only',reason='Cross-host fixture',functional_checks=['dxgi_adapters'])}}if inventory_only else{}
        return B.build_image(packages,dl,{'kf_dxprobe.exe':tool},root/'out',tc)

    def test_inventory_only_program_has_stable_identity_marker(self):
        with tempfile.TemporaryDirectory()as tmp:
            root=Path(tmp);a=self.build(root/'a',b'a',True);b=self.build(root/'b',b'other-size',True)
            self.assertEqual(a['files']['KFAPPS.ID']['sha256'],b['files']['KFAPPS.ID']['sha256'])
            self.assertEqual(R.compare(a,b)['level1'],'equal')
            self.assertNotEqual(a['files']['tools/kf_dxprobe.exe']['sha256'],b['files']['tools/kf_dxprobe.exe']['sha256'])

    def test_strict_program_difference_remains_level1_failure(self):
        with tempfile.TemporaryDirectory()as tmp:
            root=Path(tmp);a=self.build(root/'a',b'a',False);b=self.build(root/'b',b'b',False)
            self.assertEqual(R.compare(a,b)['level1'],'different')
            self.assertNotEqual(a['files']['KFAPPS.ID']['sha256'],b['files']['KFAPPS.ID']['sha256'])

    def test_inventory_only_same_size_variance_has_stable_marker(self):
        with tempfile.TemporaryDirectory()as tmp:
            root=Path(tmp);a=self.build(root/'a',b'a',True);b=self.build(root/'b',b'b',True)
            self.assertEqual(a['files']['KFAPPS.ID']['sha256'],b['files']['KFAPPS.ID']['sha256'])
            self.assertEqual(R.compare(a,b)['level1'],'equal')

    def test_inventory_only_does_not_waive_own_file_integrity(self):
        with tempfile.TemporaryDirectory()as tmp:
            root=Path(tmp);a=self.build(root,b'a',True);iso=root/'out/winapps-open.iso'
            with I.Reader(iso)as image:offset=image.files['tools/kf_dxprobe.exe'][0]
            with iso.open('r+b')as image:image.seek(offset);image.write(b'b')
            a['image']['sha256']=B.sha256(iso)
            with self.assertRaisesRegex(ValueError,'candidate file'):R.read_candidate(iso,a)

    def test_compare_command_warns_for_container_date_only(self):
        with tempfile.TemporaryDirectory()as tmp:
            root=Path(tmp);a=self.build(root,b'a',False);iso=root/'out/winapps-open.iso'
            reference=root/'reference.json';reference.write_bytes(B.canonical(a))
            data=bytearray(iso.read_bytes());data[16*I.BLOCK+813]^=1;iso.write_bytes(data)
            a['image']['sha256']=B.sha256(iso);(root/'out/winapps-open.manifest.json').write_bytes(B.canonical(a))
            with patch('sys.stdout',new_callable=io.StringIO)as out:
                self.assertEqual(B.main(['compare-image','--ci-build','--iso',str(iso),'--receipt',str(reference),'--out',str(root/'out')]),0)
            self.assertIn('WARNING LEVEL2_CONTAINER_DIFF',out.getvalue())

    def test_compare_command_fails_for_vendor_bytes(self):
        with tempfile.TemporaryDirectory()as tmp:
            root=Path(tmp);a=self.build(root,b'a',False);iso=root/'out/winapps-open.iso'
            reference=root/'reference.json';reference.write_bytes(B.canonical(a));name='pkg/sevenzip/7zr.exe'
            with I.Reader(iso)as image:offset=image.files[name][0]
            with iso.open('r+b')as image:image.seek(offset);image.write(b'X')
            a['image']['sha256']=B.sha256(iso)
            with I.Reader(iso)as image:a['files'][name].update(image.records()[name])
            (root/'out/winapps-open.manifest.json').write_bytes(B.canonical(a))
            with patch('sys.stdout',new_callable=io.StringIO):
                self.assertEqual(B.main(['compare-image','--ci-build','--iso',str(iso),'--receipt',str(reference),'--out',str(root/'out')]),1)


class Drift(unittest.TestCase):
    def gone(self,code):
        pin=dict(name='original.zip',url='https://vendor.invalid/fixed-1.zip',size=1,sha256='a'*64)
        error=urllib.error.HTTPError(pin['url'],code,'provider body must not be logged',{},None)
        with tempfile.TemporaryDirectory()as tmp,patch.object(B.urllib.request,'urlopen',side_effect=error):
            with self.assertRaisesRegex(B.InputDrift,'INPUT_DRIFT package sevenzip'):
                B.fetch_file('sevenzip',pin,Path(tmp)/'original.zip',retries=1)

    def test_vendor_404_is_input_drift(self):self.gone(404)
    def test_vendor_410_is_input_drift(self):self.gone(410)

    def test_latest_binary_url_refused(self):
        with self.assertRaises(ValueError):B.validate_pin('sevenzip',dict(name='file.zip',url='https://vendor.invalid/latest/file.zip',size=1,sha256='a'*64))

    def test_latest_compiler_url_refused(self):
        with self.assertRaises(ValueError):B.validate_pin('zig',dict(name='zig.zip',url='https://vendor.invalid/LATEST/zig.zip',size=1,sha256='a'*64))
