# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import copy
import io
import json
import os
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest.mock import patch
import zipfile

import helpers as H
import package_bundle as B
import bundle_iso as I
import bundle_compliance as C
import bundle_wheels as W
import publish_bundle as P
import bundle_repro as R
import warnings
from test_bundle_compliance import package, pin


class Classes(unittest.TestCase):
    def setUp(self):
        self.man = H.load_manifest(); self.profile = B.load(B.HERE / 'gate_profiles.json')

    def test_public_contains_only_public_ok(self):
        selected = B.select_packages(self.man, self.profile)
        self.assertTrue(selected)
        self.assertTrue(all(p['compliance']['distribution'] == 'public-ok' for p in selected))
        self.assertNotIn('pywheels', {p['id'] for p in selected})

    def test_official_catalogue_matches_only_runtime_packages(self):
        data=B.load(B.HERE/'official_fetch.json')
        self.assertEqual(set(data['packages']),set(self.profile['official_packages']))
        self.assertTrue(all(p.get('class')=='official-fetch'for p in self.man['packages']if p['id']in data['packages']))

    def test_explicit_cuda_refused_public(self):
        with self.assertRaises(ValueError): B.select_packages(self.man, self.profile, ['cuda_cudart'])

    def test_explicit_conditional_refused_public(self):
        with self.assertRaises(ValueError): B.select_packages(self.man, self.profile, ['vulkan_rt'])

    def test_private_selection_cannot_create_release_media(self):
        with self.assertRaises(ValueError):B.select_packages(self.man,self.profile,bundle_class='private')

    def test_transient_selection_cannot_create_release_media(self):
        with self.assertRaises(ValueError):B.select_packages(self.man,self.profile,bundle_class='transient-official')

    def reject_image_class(self,cls):
        with tempfile.TemporaryDirectory()as tmp:
            root=Path(tmp);out=root/'out'
            with self.assertRaises(ValueError):B.build_image([],root/'downloads',{},out,{},bundle_class=cls)
            self.assertFalse(out.exists())

    def test_private_image_refused_before_writing(self):self.reject_image_class('private')
    def test_transient_image_must_use_run_context(self):self.reject_image_class('transient-official')

    def test_private_ci_requires_explicit_build(self):
        with patch.dict(os.environ, {'CI':'true'}, clear=True), patch('sys.stderr', new_callable=io.StringIO):
            self.assertEqual(B.main(['check','--class','private']), 1)

    def test_private_actions_without_ci_refused(self):
        with patch.dict(os.environ, {'GITHUB_ACTIONS':'true'}, clear=True), patch('sys.stderr', new_callable=io.StringIO):
            self.assertEqual(B.main(['check','--class','private']), 1)

    def test_public_private_partition_disjoint(self):
        public = {p['id'] for p in B.select_packages(self.man, self.profile)}
        private = set(self.profile['official_packages'])
        self.assertFalse(public & private)
        self.assertEqual(public | private, set(self.profile['open_packages']) | set(self.profile['official_packages']))

    def test_hf_public_wheels_audited(self):
        p = next(p for p in self.man['packages'] if p['id']=='hf_wheels')
        for f in p['files']:
            self.assertEqual(f['public_audit']['archive_sha256'], f['sha256'])
            self.assertTrue(f['public_audit']['no_nvidia_libraries'])
            self.assertNotIn('MPL', p['compliance']['components'][f['name']]['license_id'])

    def test_publish_requires_configured_custom_domain(self):
        with self.assertRaises(ValueError): P.public_base({})

    def test_s3_endpoint_is_not_public_custom_domain(self):
        with self.assertRaises(ValueError): P.public_base({'BUNDLE_PUBLIC_BASE_URL':'https://account.r2.cloudflarestorage.com'})

    def test_nonpermissive_hf_wheels_extra(self):
        p = next(p for p in self.man['packages'] if p['id']=='hf_wheels_extra')
        self.assertTrue(any(f['name'].startswith('certifi-') for f in p['files']))
        self.assertEqual(p['compliance']['distribution'],'private-only')

    def test_app_private_requirements_derived(self):
        private = set(self.profile['official_packages'])
        needs = {p['id']:p.get('needs',[]) for p in self.man['packages']}
        import gate_profiles as G
        for app in H.load_apps()['apps']:
            want = bool(G.closure(app['pkgs'] + app.get('official_pkgs', []), needs) & private)
            self.assertEqual(bool((app.get('requires')or{}).get('official_fetch')), want, app['id'])

    def test_cuda_cannot_be_relabelled_public(self):
        p=next(p for p in self.man['packages'] if p['id']=='cuda_cudart')
        p['compliance']['distribution']='public-ok';self.profile['open_packages'].append(p['id']);self.profile['official_packages'].remove(p['id'])
        with self.assertRaises(ValueError):B.select_packages(self.man,self.profile,[p['id']])

    def test_runtime_cannot_be_relabelled_public(self):
        p=next(p for p in self.man['packages'] if p['id']=='vulkan_rt')
        p['compliance']['distribution']='public-ok';self.profile['open_packages'].append(p['id']);self.profile['official_packages'].remove(p['id'])
        with self.assertRaises(ValueError):B.select_packages(self.man,self.profile,[p['id']])


class Images(unittest.TestCase):
    def image(self, root, cls):
        if cls!='public':return dict(bundle_class='transient-official',packages=[],inputs='a'*64,image=dict(sha256='b'*64,size=1))
        p=package()
        dl=root/'dl'; (dl/'sevenzip').mkdir(parents=True,exist_ok=True)
        (dl/'sevenzip/LICENSE.txt').write_bytes(b'licence text')
        return B.build_image([p], dl, {}, root/cls, {}, bundle_class=cls)

    def test_both_labels_and_filenames(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)
            for cls,label,name in [('public','KFAPPS','winapps-open.iso')]:
                r=self.image(root,cls)
                self.assertEqual(r['image']['file'],name);self.assertEqual(r['bundle_class'],cls)
                with I.Reader(root/cls/name) as image:self.assertEqual(image.label,label)
                B.verify_image(root/cls/name,r)

    def test_transient_writer_uses_second_label(self):
        with tempfile.TemporaryDirectory()as tmp:
            iso=Path(tmp)/'official-run.iso';I.write_image({'original.zip':b'fixture'},iso,label='KFAPPSP')
            with I.Reader(iso)as image:self.assertEqual(image.label,'KFAPPSP')

    def test_both_classes_deterministic(self):
        with tempfile.TemporaryDirectory() as tmp:
            for cls in ('public',):
                root=Path(tmp)/cls;old=os.umask(0o022);before={k:os.environ.get(k)for k in ('TZ','LC_ALL')}
                try:
                    os.environ.update(TZ='Pacific/Honolulu',LC_ALL='C');a=self.image(root/'first',cls);os.umask(0o077)
                    os.environ.update(TZ='Asia/Tokyo',LC_ALL='C.UTF-8');b=self.image(root/'second',cls)
                finally:
                    os.umask(old)
                    for k,v in before.items():
                        if v is None:os.environ.pop(k,None)
                        else:os.environ[k]=v
                result=R.compare(a,b);self.assertEqual(result['level1'],'equal')
                if result['level2']!='equal':warnings.warn('LEVEL2_CONTAINER_DIFF '+json.dumps(result['container_diff']))

    def test_private_receipt_cannot_publish(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);r=self.image(root,'private')
            with self.assertRaises(ValueError): P.publish(None,root/'private/winapps-private.iso',r,root/'state')

    def test_private_sources_cannot_publish(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);r=self.image(root,'private')
            with self.assertRaises(ValueError): P.publish_sources(None,root/'private/winapps-private.iso',r,root/'state',P.DEFAULT_PART)


class Wheels(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.path=Path(self.tmp.name)/'test.whl'

    def wheel(self, licence='MIT', files=None):
        with zipfile.ZipFile(self.path,'w')as z:
            z.writestr('test-1.0.dist-info/METADATA','Name: test\nVersion: 1.0\nLicense-Expression: '+licence+'\n')
            for n,b in (files or {}).items():z.writestr(n,b)
        return pin('test.whl',self.path.read_bytes()),dict(version='1.0',license_id=licence)

    def test_nvidia_library_filename_refused(self):
        f,c=self.wheel(files={'lib/cudart64_12.dll':b'MZdata'})
        with self.assertRaises(ValueError):W.inspect(self.path,f,c)

    def test_nvidia_reference_in_native_binary_refused(self):
        f,c=self.wheel(files={'module.pyd':b'MZ\0nvrtc64_120_0.dll\0'})
        with self.assertRaises(ValueError):W.inspect(self.path,f,c)

    def test_cublas_reference_in_generic_binary_refused(self):
        f,c=self.wheel(files={'native.pyd':b'MZ\0cublas64_12.dll\0'})
        with self.assertRaises(ValueError):W.inspect(self.path,f,c)

    def test_mpl_wheel_refused(self):
        f,c=self.wheel('MPL-2.0')
        with self.assertRaises(ValueError):W.inspect(self.path,f,c)

    def test_gpl_wheel_refused(self):
        f,c=self.wheel('GPL-3.0-only')
        with self.assertRaises(ValueError):W.inspect(self.path,f,c)

    def test_operator_is_not_a_licence(self): self.assertFalse(W.permissive('OR'))
    def test_broken_expression_is_not_permissive(self): self.assertFalse(W.permissive('MIT OR'))

    def legacy(self, value):
        with zipfile.ZipFile(self.path,'w')as z:
            z.writestr('test-1.0.dist-info/METADATA','Name: test\nVersion: 1.0\nLicense: '+value+'\nClassifier: License :: OSI Approved :: Apache Software License\n')
        return pin('test.whl',self.path.read_bytes()),dict(version='1.0',license_id='Apache-2.0')

    def test_unknown_legacy_licence_is_private(self):
        f,c=self.legacy('See separate agreement')
        with self.assertRaises(ValueError):W.inspect(self.path,f,c)

    def test_proprietary_legacy_licence_is_private(self):
        f,c=self.legacy('Proprietary')
        with self.assertRaises(ValueError):W.inspect(self.path,f,c)

    def test_reviewed_metadata_hash_required(self):
        f,c=self.wheel();f['public_audit']=W.inspect(self.path,f,c);f['public_audit']['metadata_sha256']='0'*64
        with self.assertRaises(ValueError):W.verify(self.path,f,c)

    def test_reviewed_inventory_required(self):
        f,c=self.wheel();f['public_audit']=W.inspect(self.path,f,c);f['public_audit']['member_count']+=1
        with self.assertRaises(ValueError):W.verify(self.path,f,c)


class MissingPrivate(unittest.TestCase):
    def run_missing(self, configured, spoofed=False):
        import winapps
        from unittest.mock import Mock
        with tempfile.TemporaryDirectory()as tmp:
            app=next(a for a in H.load_apps()['apps']if a['id']=='vectorAdd_demo')
            vm=Mock();vm.gsp_cycles.return_value=0
            g=winapps.GuestSession(winapps.Cfg(official_iso='private.iso'if configured else None),vm,H.load_apps(),H.load_manifest(),tmp,lambda x:None,'fixture')
            g.official_attached=configured and not spoofed
            g.official_cd="P:" if spoofed else None
            g._stage=Mock(side_effect=AssertionError('private package must not stage'))
            r=g.run_app(app);self.assertEqual(r['verdict'],'NOTRUN');self.assertIn('missing-official-media',r['line']);g._stage.assert_not_called()

    def test_absent_private_disk_is_notrun(self):self.run_missing(False)
    def test_private_path_without_mounted_label_is_notrun(self):self.run_missing(True)

    def test_guest_label_without_host_attachment_is_notrun(self):self.run_missing(False, True)
    def test_configured_path_without_attachment_is_notrun(self):self.run_missing(True, True)

    def test_public_profile_has_no_private_requirement(self):
        import gate_profiles as G
        byid={a['id']:a for a in H.load_apps()['apps']}
        self.assertTrue(G.resolve('public'))
        self.assertTrue(all(not byid[x].get('requires')for x in G.resolve('public')))


class MediaGrading(unittest.TestCase):
    def classify(self, before, after):
        source=(Path(H.REPO)/'scripts/bench/box/windows_apps.sh').read_text()
        body=source[source.index('classify() {'):source.index('\nrun_loop_alive_any()')]
        return subprocess.check_output(['bash','-c',body+'\nclassify RETRY '+before+' '+after+' 0 0 1'],text=True).strip()

    def test_missing_private_with_green_host_is_media(self):self.assertEqual(self.classify('green','green'),'RETRY MEDIA')
    def test_missing_private_with_host_fault_is_still_no_pass(self):self.assertEqual(self.classify('RED','RED'),'RETRY MEDIA')


@unittest.skipUnless(H.find_pwsh(),'PowerShell required for volume assignment fixtures')
class DriveLetters(unittest.TestCase):
    def mapping(self, extra):
        source=(Path(H.AM)/'guest/kf_guest_setup.ps1').read_text()
        block=source[source.index('$officialMappingFailed ='):source.index('$officialCd =')]
        with tempfile.TemporaryDirectory()as tmp:
            root=Path(tmp);script=root/'map.ps1'
            script.write_text("$ExpectOfficial=$false\n$info=@{notes=@()}\n$global:volumes=@([pscustomobject]@{Label='KFAPPS';DriveLetter='E:'},[pscustomobject]@{Label='KFAPPSP';DriveLetter='F:'}"+extra+")\nfunction Get-CimInstance {param($ClassName) $global:volumes}\nfunction Set-CimInstance {param($InputObject,$Property) $InputObject.DriveLetter=$Property.DriveLetter}\n"+block+"\n@{ volumes=$global:volumes; private_failed=$officialMappingFailed } | ConvertTo-Json -Depth 5 -Compress\n")
            env=dict(os.environ,XDG_CONFIG_HOME=str(root/'config'),XDG_CACHE_HOME=str(root/'cache'),DOTNET_CLI_HOME=str(root/'dotnet'),POWERSHELL_TELEMETRY_OPTOUT='1')
            return subprocess.run([H.find_pwsh(),'-NoProfile','-NonInteractive','-File',str(script)],env=env,capture_output=True,text=True,timeout=30)

    def test_both_labels_receive_fixed_letters(self):
        r=self.mapping('');self.assertEqual(r.returncode,0,r.stderr)
        d=json.loads(r.stdout);self.assertEqual([v['DriveLetter']for v in d['volumes']],['K:','P:'])

    def test_public_conflict_is_refused(self):
        r=self.mapping(",[pscustomobject]@{Label='OTHER';DriveLetter='K:'}");self.assertEqual(r.returncode,3)

    def test_private_conflict_does_not_break_public_disk(self):
        r=self.mapping(",[pscustomobject]@{Label='OTHER';DriveLetter='P:'}");self.assertEqual(r.returncode,0,r.stderr)
        d=json.loads(r.stdout);self.assertTrue(d['private_failed']);self.assertEqual(d['volumes'][0]['DriveLetter'],'K:')


class PublicFetch(unittest.TestCase):
    image = Images.image
    def test_custom_domain_fetch_has_no_auth_or_worker_fallback(self):
        import fetch_public_bundle as F
        import shutil
        with tempfile.TemporaryDirectory()as tmp:
            root=Path(tmp);receipt=self.image(root,'public');out=root/'fetched.iso'
            def copy(package,pin,destination,**kwargs):
                self.assertTrue(kwargs['image']);self.assertIn('/winapps-open/kfapps-',pin['url'])
                shutil.copyfile(root/'public/winapps-open.iso',destination)
            with patch.object(F.urllib.request,'urlopen',return_value=io.BytesIO(B.canonical(receipt)))as request,patch.object(B,'fetch_file',side_effect=copy):
                F.fetch('https://bundles.invalid',B.pin_text(receipt['image']['sha256'],receipt['inputs']),out)
            self.assertNotIn('Authorization',request.call_args.args[0].headers)
            self.assertTrue(out.exists());self.assertTrue((root/'winapps-open.manifest.json').exists())

    def test_private_manifest_is_rejected_before_download(self):
        import fetch_public_bundle as F
        with tempfile.TemporaryDirectory()as tmp:
            root=Path(tmp);receipt=self.image(root,'private')
            with patch.object(F.urllib.request,'urlopen',return_value=io.BytesIO(B.canonical(receipt))),patch.object(B,'fetch_file')as download:
                with self.assertRaises(ValueError):F.fetch('https://bundles.invalid',B.pin_text(receipt['image']['sha256'],receipt['inputs']),root/'bad.iso')
            download.assert_not_called()

    def test_manifest_wrong_pin_is_rejected_before_download(self):
        import fetch_public_bundle as F
        with tempfile.TemporaryDirectory()as tmp:
            root=Path(tmp);receipt=self.image(root,'public')
            with patch.object(F.urllib.request,'urlopen',return_value=io.BytesIO(B.canonical(receipt))),patch.object(B,'fetch_file')as download:
                with self.assertRaises(ValueError):F.fetch('https://bundles.invalid',B.pin_text('0'*64,receipt['inputs']),root/'bad.iso')
            download.assert_not_called()
