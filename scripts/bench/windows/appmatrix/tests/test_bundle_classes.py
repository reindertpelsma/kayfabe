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


class PublicBaseDefault(unittest.TestCase):
    """The first real-box run (2026-11-10) passed an EMPTY --base because nothing set KF_WINAPPS_PUBLIC_BASE_URL: the fetch
    never started (RETRY MEDIA after 6 s). Falsifier: any of these passes on `--base ''` reaching the fetcher, on a default that
    is not the bundles domain, or on a base other than the bundles domain reaching a request."""
    image = Images.image
    other_bases = ['https://evil.example', 'http://bundles.kayfabe.dev', 'https://bundles.kayfabe.dev.evil.example',
                   'https://user@bundles.kayfabe.dev', 'https://bundles.kayfabe.dev:8443', 'https://bundles.kayfabe.dev/x',
                   'https://bundles.kayfabe.dev?x=1', 'https://account.r2.dev', 'https://bundles.invalid', 'bundles.kayfabe.dev']

    def main(self, root, receipt, *base):
        import fetch_public_bundle as F
        import shutil
        pin = root / 'bundle.pin'
        pin.write_text(B.pin_text(receipt['image']['sha256'], receipt['inputs']))
        errors = io.StringIO()
        requests = []

        def opened(request, timeout=None):
            requests.append(request.full_url)
            return io.BytesIO(B.canonical(receipt))

        def copy(package, pin_, destination, **kwargs):
            requests.append(pin_['url'])
            shutil.copyfile(root / 'public/winapps-open.iso', destination)

        with patch.object(F.urllib.request, 'urlopen', side_effect=opened), patch.object(F.urllib.request, 'install_opener'), \
                patch.object(B, 'fetch_file', side_effect=copy), patch('sys.stderr', errors):
            rc = F.main([*base, '--pin', str(pin), '--out', str(root / 'out.iso')])
        return rc, requests, errors.getvalue()

    def test_default_base_is_the_bundles_domain_and_is_fetched(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            receipt = self.image(root, 'public')
            digest = receipt['image']['sha256']
            rc, requests, _ = self.main(root, receipt)
            self.assertEqual(rc, 0)
            self.assertEqual(requests, ['https://bundles.kayfabe.dev/winapps-open/manifest-%s.json' % digest,
                                        'https://bundles.kayfabe.dev/winapps-open/kfapps-%s.iso' % digest])

    def test_the_repo_pin_names_objects_that_the_default_base_serves(self):
        import fetch_public_bundle as F
        pin = (Path(H.AM) / 'bundle.pin').read_text()
        digest = [line.split('sha256=')[1] for line in pin.splitlines() if line.startswith('winapps-open.iso ')][0]
        self.assertEqual(P.keys(digest), ('winapps-open/kfapps-%s.iso' % digest, 'winapps-open/manifest-%s.json' % digest))
        self.assertEqual(F.resolve_base(None), 'https://bundles.kayfabe.dev')
        self.assertEqual(P.public_base({'BUNDLE_PUBLIC_BASE_URL': F.resolve_base(None)}), 'https://bundles.kayfabe.dev')

    def test_an_empty_base_is_refused_before_any_request(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            receipt = self.image(root, 'public')
            rc, requests, errors = self.main(root, receipt, '--base', '')
            self.assertEqual((rc, requests), (1, []))
            self.assertIn('base-is-not-the-bundles-domain', errors)

    def test_no_other_host_or_form_of_the_base_reaches_a_request(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            receipt = self.image(root, 'public')
            for base in self.other_bases:
                with self.subTest(base=base):
                    rc, requests, errors = self.main(root, receipt, '--base', base)
                    self.assertEqual((rc, requests), (1, []))
                    self.assertIn('base-is-not-the-bundles-domain', errors)

    def test_the_bundles_domain_itself_may_be_named_with_a_trailing_slash(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            receipt = self.image(root, 'public')
            for base in ('https://bundles.kayfabe.dev', 'https://bundles.kayfabe.dev/'):
                rc, requests, _ = self.main(root, receipt, '--base', base)
                self.assertEqual(rc, 0)
                self.assertTrue(requests[0].startswith('https://bundles.kayfabe.dev/winapps-open/manifest-'), requests)


class WinAppsStageWiring(unittest.TestCase):
    """windows_apps.sh must reach the fetcher without an empty --base, and must select a driver the catalogue pins."""

    @property
    def script(self):
        return (Path(H.REPO) / 'scripts/bench/box/windows_apps.sh').read_text()

    def fetch_argv(self, **env):
        body = self.script
        body = body[body.index('fetch_app_disk() {'):body.index('\nrun_loop_alive_any()')]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'fetch_public_bundle.py').write_text('import json,sys\nprint(json.dumps(sys.argv[1:]))\n')
            e = {'PATH': os.environ['PATH'], **env}
            subprocess.run(['bash', '-c', body + '\nAPPM=%s\nfetch_app_disk out.iso %s/log' % (root, root)], env=e, check=True, timeout=30)
            return json.loads((root / 'log').read_text())

    def test_unset_base_is_not_passed_on_as_an_empty_base(self):
        argv = self.fetch_argv()
        self.assertNotIn('--base', argv)
        self.assertNotIn('', argv)
        self.assertEqual(argv[-2:], ['--out', 'out.iso'])

    def test_an_empty_base_variable_is_not_passed_on_either(self):
        argv = self.fetch_argv(KF_WINAPPS_PUBLIC_BASE_URL='')
        self.assertNotIn('--base', argv)
        self.assertNotIn('', argv)

    def test_a_set_base_is_passed_on_for_the_fetcher_to_validate(self):
        argv = self.fetch_argv(KF_WINAPPS_PUBLIC_BASE_URL='https://bundles.kayfabe.dev')
        self.assertEqual(argv[:2], ['--base', 'https://bundles.kayfabe.dev'])

    def test_the_stage_script_selftest_is_green_with_and_without_the_variable(self):
        script = Path(H.REPO) / 'scripts/bench/box/windows_apps.sh'
        for env in ({}, {'KF_WINAPPS_PUBLIC_BASE_URL': ''}):
            r = subprocess.run(['bash', str(script), '--selftest'], env={'PATH': os.environ['PATH'], 'HOME': tempfile.gettempdir(), **env}, capture_output=True, text=True, timeout=60)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertIn('WINAPPS_SELFTEST_OK', r.stdout)

    def test_the_default_official_driver_selects_every_open_profile(self):
        import re
        import official_runtime as O
        import gate_profiles as G
        default = re.search(r'^DEFAULT_OFFICIAL_DRIVER=([0-9.]+)', self.script, re.M).group(1)
        catalogue = O.validate(B.load(B.HERE / 'official_fetch.json'))
        apps = {a['id']: a for a in H.load_apps()['apps']}
        for profile in ('small', 'mid', 'full'):
            with self.subTest(profile=profile):
                chosen = [apps[x] for x in G.resolve(profile)]
                self.assertTrue(O.select(catalogue, chosen, default))
                if any((a.get('requires') or {}).get('official_fetch') for a in chosen):
                    with self.assertRaises(O.MediaDrift):  # why an unset driver was the next wall
                        O.select(catalogue, chosen, None)


class PublicFetchReasons(unittest.TestCase):
    """2026-11-19/20 (MEASURED): a pin that no longer matched the tree failed as `PUBLIC_BUNDLE_FAIL (no credentials or provider
    body logged)` and was graded a retryable MEDIA problem, so the cause was invisible and a fresh box was rented for nothing.
    FALSIFIER: main() prints the exception text or a provider body; a reason outside the closed set; a recipe/pin mismatch of
    verify_image that is not named; a classification that a provider-controlled message can steer."""
    RECIPE = ('embedded recipe input identity mismatch', 'artifact recipe differs from reviewed checkout inputs',
              'recipe source audit differs from reviewed checkout')
    PIN = ('public manifest differs from repo pin', 'image size/sha256 mismatch')
    HOSTILE = ' https://u:p@bundles.kayfabe.dev/x?X-Amz-Signature=deadbeef Authorization: Bearer t0ken <html>provider body</html>'

    def attempt(self, error):
        import fetch_public_bundle as F
        errors, out = io.StringIO(), io.StringIO()
        with tempfile.TemporaryDirectory() as tmp, patch.object(F, 'fetch', side_effect=error), \
                patch.object(F.urllib.request, 'install_opener'), patch('sys.stderr', errors), patch('sys.stdout', out):
            pin = Path(tmp) / 'bundle.pin'
            pin.write_text('')
            rc = F.main(['--pin', str(pin), '--out', str(Path(tmp) / 'out.iso')])
        return rc, errors.getvalue(), out.getvalue()

    def assert_reason(self, error, reason):
        rc, err, out = self.attempt(error)
        self.assertEqual((rc, err, out), (1, 'PUBLIC_BUNDLE_FAIL reason=' + reason + '\n', ''))

    def test_the_first_recipe_mismatch_is_named(self): self.assert_reason(ValueError(self.RECIPE[0]), 'RECIPE_INPUT_DRIFT')
    def test_the_reviewed_inputs_mismatch_is_named(self): self.assert_reason(ValueError(self.RECIPE[1]), 'RECIPE_INPUT_DRIFT')
    def test_the_recipe_source_audit_mismatch_is_named(self): self.assert_reason(ValueError(self.RECIPE[2]), 'RECIPE_INPUT_DRIFT')
    def test_a_manifest_that_differs_from_the_pin_is_named(self): self.assert_reason(ValueError(self.PIN[0]), 'PIN_MISMATCH')
    def test_an_image_size_or_sha_mismatch_is_named(self): self.assert_reason(ValueError(self.PIN[1]), 'PIN_MISMATCH')

    def test_an_oserror_is_network_and_its_text_is_not_printed(self):
        self.assert_reason(OSError('connection reset' + self.HOSTILE), 'NETWORK')

    def test_a_url_error_is_network_and_its_text_is_not_printed(self):
        import urllib.error
        self.assert_reason(urllib.error.URLError('timed out' + self.HOSTILE), 'NETWORK')

    def test_any_other_error_is_other_and_its_text_is_not_printed(self):
        for error in (ValueError('something else' + self.HOSTILE), KeyError('image'), TypeError('x' + self.HOSTILE)):
            with self.subTest(error=type(error).__name__):
                self.assert_reason(error, 'OTHER')

    def test_a_provider_message_that_merely_contains_a_recipe_phrase_is_other(self):
        self.assert_reason(ValueError('provider said: ' + self.RECIPE[0] + self.HOSTILE), 'OTHER')

    def test_a_provider_message_that_merely_contains_a_pin_phrase_is_other(self):
        self.assert_reason(ValueError('package public-bundle/x: ' + self.PIN[0]), 'OTHER')

    def test_a_wrapped_download_error_with_a_recipe_phrase_is_other(self):
        self.assert_reason(ValueError('package public-bundle/winapps-open.iso: ' + self.RECIPE[1]), 'OTHER')

    def test_a_message_that_ends_with_a_pin_phrase_is_other(self):
        self.assert_reason(ValueError('retry later: ' + self.PIN[1]), 'OTHER')

    def test_the_closed_set_is_the_only_output(self):
        import fetch_public_bundle as F
        for error in (ValueError(self.RECIPE[0]), ValueError(self.PIN[1]), OSError('x'), ValueError('y'), KeyError('z')):
            self.assertRegex(self.attempt(error)[1], r'\APUBLIC_BUNDLE_FAIL reason=(RECIPE_INPUT_DRIFT|PIN_MISMATCH|NETWORK|OTHER)\n\Z')
        self.assertEqual(F.REASONS, ('RECIPE_INPUT_DRIFT', 'PIN_MISMATCH', 'NETWORK', 'OTHER'))

    def test_the_phrases_are_the_ones_verify_image_raises(self):
        # package_bundle.py is a hashed recipe input and cannot export its messages; a reworded message would silently become OTHER
        import fetch_public_bundle as F
        raised = {B.__file__: self.RECIPE + self.PIN[1:], F.__file__: self.PIN[:1]}
        for path, messages in raised.items():
            source = Path(path).read_text()
            for message in messages:
                self.assertEqual(source.count("raise ValueError('%s')" % message) + source.count('raise ValueError("%s")' % message), 1, message)

    def test_success_is_unchanged(self):
        import fetch_public_bundle as F
        out = io.StringIO()
        with tempfile.TemporaryDirectory() as tmp, patch.object(F, 'fetch'), patch.object(F.urllib.request, 'install_opener'), patch('sys.stdout', out):
            pin = Path(tmp) / 'bundle.pin'
            pin.write_text('')
            self.assertEqual(F.main(['--pin', str(pin), '--out', str(Path(tmp) / 'out.iso')]), 0)
        self.assertEqual(out.getvalue(), 'PUBLIC_BUNDLE_OK\n')


class WinAppsPinRefusal(unittest.TestCase):
    """A fetch that failed because the pin does not match this checkout is a named refusal, not RETRY class=MEDIA: another box
    cannot fix it. FALSIFIER: RECIPE_INPUT_DRIFT or PIN_MISMATCH reaching `verdict RETRY MEDIA`; any other reason (NETWORK, OTHER,
    a reason the log merely contains) becoming a refusal; a refusal that exits 0 or 3 or prints another line."""

    @property
    def body(self):
        text = (Path(H.REPO) / 'scripts/bench/box/windows_apps.sh').read_text()
        return text[text.index('fetch_app_disk() {'):text.index('\nrun_loop_alive_any()')]

    def run_fn(self, log_text, call, log_name='fetch_bundle.txt'):
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / log_name
            log.write_text(log_text)
            script = ('say() { echo "$*"; }\nverdict() { echo "VERDICT $*"; }\n' + self.body + '\nVERDICT=""\nrc_final=0\n'
                      + call.replace('LOG', "'%s'" % log) + '\necho "FELL_THROUGH"\n')
            r = subprocess.run(['bash', '-c', script], env={'PATH': os.environ['PATH']}, capture_output=True, text=True, timeout=30)
            return r.returncode, r.stdout, r.stderr

    def reason(self, log_text):
        rc, out, _ = self.run_fn(log_text, 'pin_refusal_reason LOG')
        return rc, out

    def test_a_recipe_drift_log_names_its_reason(self):
        self.assertEqual(self.reason('FETCH public-bundle/winapps-open.iso bytes=1\nPUBLIC_BUNDLE_FAIL reason=RECIPE_INPUT_DRIFT\n'),
                         (0, 'RECIPE_INPUT_DRIFT\nFELL_THROUGH\n'))

    def test_a_pin_mismatch_log_names_its_reason(self):
        self.assertEqual(self.reason('PUBLIC_BUNDLE_FAIL reason=PIN_MISMATCH\n'), (0, 'PIN_MISMATCH\nFELL_THROUGH\n'))

    def test_a_network_failure_is_not_a_pin_reason(self):
        self.assertEqual(self.reason('PUBLIC_BUNDLE_FAIL reason=NETWORK\n')[1], 'FELL_THROUGH\n')

    def test_an_other_failure_is_not_a_pin_reason(self):
        self.assertEqual(self.reason('PUBLIC_BUNDLE_FAIL reason=OTHER\n')[1], 'FELL_THROUGH\n')

    def test_a_log_without_a_failure_line_is_not_a_pin_reason(self):
        self.assertEqual(self.reason('FETCH public-bundle/winapps-open.iso bytes=1\n')[1], 'FELL_THROUGH\n')

    def test_a_reason_with_trailing_text_is_not_a_pin_reason(self):
        for line in ('PUBLIC_BUNDLE_FAIL reason=PIN_MISMATCH extra', 'PUBLIC_BUNDLE_FAIL reason=RECIPE_INPUT_DRIFT_X',
                     'x PUBLIC_BUNDLE_FAIL reason=PIN_MISMATCH', 'PUBLIC_BUNDLE_FAIL reason=pin_mismatch'):
            with self.subTest(line=line):
                self.assertEqual(self.reason(line + '\n')[1], 'FELL_THROUGH\n')

    def test_a_missing_log_is_not_a_pin_reason(self):
        rc, out, _ = self.run_fn('', 'pin_refusal_reason /nonexistent/fetch_bundle.txt')
        self.assertEqual(out, 'FELL_THROUGH\n')

    def test_the_recipe_drift_refusal_line_and_exit_code(self):
        rc, out, _ = self.run_fn('PUBLIC_BUNDLE_FAIL reason=RECIPE_INPUT_DRIFT\n', 'fetch_failed LOG')
        self.assertEqual(rc, 2)
        self.assertEqual(out.splitlines()[-1], 'WINAPPS_REFUSED class=PIN reason=RECIPE_INPUT_DRIFT')
        self.assertNotIn('VERDICT', out)
        self.assertNotIn('FELL_THROUGH', out)

    def test_the_pin_mismatch_refusal_line_and_exit_code(self):
        rc, out, _ = self.run_fn('PUBLIC_BUNDLE_FAIL reason=PIN_MISMATCH\n', 'fetch_failed LOG')
        self.assertEqual(rc, 2)
        self.assertEqual(out.splitlines()[-1], 'WINAPPS_REFUSED class=PIN reason=PIN_MISMATCH')
        self.assertNotIn('VERDICT', out)

    def test_the_refusal_says_what_to_do(self):
        _, out, _ = self.run_fn('PUBLIC_BUNDLE_FAIL reason=RECIPE_INPUT_DRIFT\n', 'fetch_failed LOG')
        self.assertIn('republish the bundle', out)
        self.assertIn('bundle.pin', out)

    def test_a_network_failure_stays_a_media_retry(self):
        rc, out, _ = self.run_fn('PUBLIC_BUNDLE_FAIL reason=NETWORK\n', 'fetch_failed LOG')
        self.assertEqual(rc, 3)
        self.assertIn('VERDICT RETRY MEDIA fetch of winapps-open.iso failed', out)
        self.assertIn('PUBLIC_BUNDLE_FAIL reason=NETWORK', out)
        self.assertNotIn('WINAPPS_REFUSED', out)

    def test_an_unnamed_failure_stays_a_media_retry(self):
        rc, out, _ = self.run_fn('boom\n', 'fetch_failed LOG')
        self.assertEqual(rc, 3)
        self.assertIn('VERDICT RETRY MEDIA', out)
        self.assertNotIn('WINAPPS_REFUSED', out)

    def test_the_stage_calls_it_on_a_failed_fetch(self):
        text = (Path(H.REPO) / 'scripts/bench/box/windows_apps.sh').read_text()
        self.assertIn('if ! fetch_app_disk "$ISO" "$EV/fetch_bundle.txt"; then\n        fetch_failed "$EV/fetch_bundle.txt"\n    fi', text)

    def test_the_media_retry_for_a_failed_fetch_is_only_inside_it(self):
        text = (Path(H.REPO) / 'scripts/bench/box/windows_apps.sh').read_text()
        marker = 'verdict RETRY MEDIA "fetch of winapps-open.iso failed'
        self.assertEqual(text.count(marker), 1)
        self.assertTrue(text.index('fetch_failed() {') < text.index(marker) < text.index('\nrun_loop_alive_any()'))
