"""GPU-free installer-pin tests; all bytes and HTTP responses are synthetic.
Falsifier: a 404, changed hash/size, corrupt cache or drift marker becomes success.
"""
import base64
import hashlib
import importlib.util
import io
import json
import tempfile
import os
import subprocess
import unittest
import urllib.error
from pathlib import Path
from unittest.mock import patch
import vast_report
import vast_payload

HERE = Path(__file__).resolve().parent
DATA = b'synthetic installer fixture, never executable'

def pin(os_name):
    version = '580.65.06' if os_name == 'linux' else '580.88'
    name = f'NVIDIA-Linux-x86_64-{version}.run' if os_name == 'linux' else f'{version}-desktop-win10-win11-64bit-international-dch-whql.exe'
    path = f'XFree86/Linux-x86_64/{version}' if os_name == 'linux' else f'Windows/{version}'
    return dict(os=os_name,version=version,status='PINNED',url=f'https://download.nvidia.com/{path}/{name}',sha256=hashlib.sha256(DATA).hexdigest(),size=len(DATA),checked='2026-11-07',source='SYNTHETIC fixture; not official installer bytes',catalog_url=None)

def encoded(value):
    return base64.urlsafe_b64encode(json.dumps(value).encode()).decode().rstrip('=')

class DriverMedia(unittest.TestCase):
    def setUp(self):
        spec=importlib.util.spec_from_file_location('j2_driver_media',HERE/'release_driver_media.py')
        self.media=importlib.util.module_from_spec(spec);spec.loader.exec_module(self.media)
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.target=Path(self.temp.name)/'installer'

    def response(self,p,data=DATA):
        r=io.BytesIO(data);r.status=200;r.headers={'Content-Length':str(len(data))};r.geturl=lambda:p['url'];return r

    def test_provision_drift_cannot_pass_even_with_zero_exit(self):
        with self.assertRaises(vast_payload.EvidenceError) as error:
            vast_payload.provision_rows('DRIVER_MEDIA_DRIFT reason=HASH\n',0)
        self.assertEqual(error.exception.name,'driver_media_drift')

    def test_provision_404_is_reported_as_media_drift(self):
        with self.assertRaises(vast_payload.EvidenceError) as error:
            vast_payload.provision_rows('DRIVER_MEDIA_DRIFT reason=HTTP_404\n',3)
        self.assertEqual(error.exception.name,'driver_media_drift')


def install_cases(os_name):
    def valid(self):
        p=pin(os_name)
        self.media.fetch_installer(p,self.target,opener=lambda request:self.response(p))
        self.assertEqual(self.target.read_bytes(),DATA)
    def missing(self):
        p=pin(os_name)
        def opener(request):raise urllib.error.HTTPError(p['url'],404,'fixture',{},None)
        with self.assertRaises(self.media.MediaDrift) as error:self.media.fetch_installer(p,self.target,opener=opener)
        self.assertEqual(error.exception.reason,'HTTP_404');self.assertFalse(self.target.exists())
    def changed_hash(self):
        p=pin(os_name);p['sha256']='0'*64
        with self.assertRaises(self.media.MediaDrift) as error:self.media.fetch_installer(p,self.target,opener=lambda request:self.response(p))
        self.assertEqual(error.exception.reason,'HASH');self.assertFalse(self.target.exists())
    def changed_size(self):
        p=pin(os_name);p['size']+=1
        with self.assertRaises(self.media.MediaDrift) as error:self.media.fetch_installer(p,self.target,opener=lambda request:self.response(p))
        self.assertEqual(error.exception.reason,'SIZE');self.assertFalse(self.target.exists())
    def corrupt_cache(self):
        p=pin(os_name);self.target.write_bytes(b'x'*len(DATA))
        with self.assertRaises(self.media.MediaDrift) as error:self.media.fetch_installer(p,self.target,opener=lambda request:self.fail('must not silently refresh corrupt cache'))
        self.assertEqual(error.exception.reason,'HASH')
    def unpinned(self):
        p=pin(os_name);p.update(status='UNPINNED',sha256=None,size=None)
        with self.assertRaises(self.media.MediaDrift) as error:self.media.fetch_installer(p,self.target,opener=lambda request:self.fail('UNPINNED must not fetch'))
        self.assertEqual(error.exception.reason,'UNPINNED')
    def redirect(self):
        p=pin(os_name);r=self.response(p);r.geturl=lambda:'https://drivers.example.com/changed'
        with self.assertRaises(self.media.MediaDrift) as error:self.media.fetch_installer(p,self.target,opener=lambda request:r)
        self.assertEqual(error.exception.reason,'REDIRECT');self.assertFalse(self.target.exists())
    def cli_drift(self):
        p=pin(os_name)
        with patch.dict('os.environ',{'KF_RELEASE_DRIVER_PINS':encoded([p])}),patch.object(self.media,'fetch_installer',side_effect=self.media.MediaDrift('HTTP_404')),patch('sys.stdout',new_callable=io.StringIO) as out:
            status=self.media.main(['--os',os_name,'--version',p['version'],'--out',str(self.target)])
        self.assertEqual(status,3);self.assertIn('DRIVER_MEDIA_DRIFT reason=HTTP_404',out.getvalue());self.assertNotIn('OK',out.getvalue())
    def cli_filesystem(self):
        p=pin(os_name)
        with patch.dict('os.environ',{'KF_RELEASE_DRIVER_PINS':encoded([p])}),patch.object(self.media,'fetch_installer',side_effect=OSError('synthetic I/O error')),patch('sys.stdout',new_callable=io.StringIO) as out:
            status=self.media.main(['--os',os_name,'--version',p['version'],'--out',str(self.target)])
        self.assertEqual(status,3);self.assertEqual(out.getvalue(),'DRIVER_MEDIA_DRIFT reason=FILESYSTEM\n')
    for name,body in locals().copy().items():
        if callable(body) and name not in ['install_cases']:
            setattr(DriverMedia,'test_'+name+'_'+os_name,body)
for os_name in ['linux','windows']:install_cases(os_name)

class DriverPinContract(unittest.TestCase):
    def test_valid_linux_and_windows_pins_are_transported(self):
        for os_name in ['linux','windows']:self.assertTrue(vast_report.driver_pins_json(encoded([pin(os_name)])))
    def test_wrong_hash_cannot_enter_environment(self):
        for os_name in ['linux','windows']:
            p=pin(os_name);p['sha256']='inferred';self.assertFalse(vast_report.driver_pins_json(encoded([p])))
    def test_unofficial_url_cannot_enter_environment(self):
        for os_name in ['linux','windows']:
            p=pin(os_name);p['url']=p['url'].replace('download.nvidia.com','drivers.example.com');self.assertFalse(vast_report.driver_pins_json(encoded([p])))
    def test_http_url_cannot_enter_environment(self):
        for os_name in ['linux','windows']:
            p=pin(os_name);p['url']=p['url'].replace('https:','http:');self.assertFalse(vast_report.driver_pins_json(encoded([p])))

class PayloadPinCoverage(unittest.TestCase):
    def environment(self):
        matrix=json.loads((HERE.parents[1]/'ci/vast-broker/release-matrix.json').read_text())
        row=matrix['dies'][1]
        plan=dict(status='proposed',subruns=row['subruns'],cross_subrun=row['cross_subrun'])
        env=dict(KF_HOST_DRIVER=row['drivers']['host_driver'],KF_LINUX_GUEST_DRIVER=row['drivers']['linux_guest_driver'],KF_WINDOWS_GUEST_DRIVER=row['drivers']['windows_guest_driver'],KF_RELEASE_SUBRUNS=encoded(plan),KF_RELEASE_DRIVER_PINS=encoded(matrix['driver_installers']))
        return env,matrix,plan
    def test_missing_pin_field_refuses_payload(self):
        env,_,_=self.environment();env.pop('KF_RELEASE_DRIVER_PINS')
        with self.assertRaises(vast_payload.EvidenceError):vast_payload.release_driver_environment(env)
    def test_missing_full_installer_refuses_payload(self):
        env,m,_=self.environment();env['KF_RELEASE_DRIVER_PINS']=encoded([p for p in m['driver_installers'] if p['version']!='580.65.06'])
        with self.assertRaises(vast_payload.EvidenceError):vast_payload.release_driver_environment(env)
    def test_missing_subrun_installer_refuses_payload(self):
        env,m,_=self.environment();env['KF_RELEASE_DRIVER_PINS']=encoded([p for p in m['driver_installers'] if p['version']!='590.48.01'])
        with self.assertRaises(vast_payload.EvidenceError):vast_payload.release_driver_environment(env)
    def test_nested_subrun_driver_is_a_checked_error(self):
        env,_,plan=self.environment();plan['subruns'][0]['drivers']=[[]];env['KF_RELEASE_SUBRUNS']=encoded(plan)
        with self.assertRaises(vast_payload.EvidenceError):vast_payload.release_driver_environment(env)
    def test_nested_cross_driver_is_a_checked_error(self):
        env,_,plan=self.environment();plan['cross_subrun']['drivers']['host_driver']=[];env['KF_RELEASE_SUBRUNS']=encoded(plan)
        with self.assertRaises(vast_payload.EvidenceError):vast_payload.release_driver_environment(env)

class ShellMediaGuards(unittest.TestCase):
    def marked(self, path, name):
        source = path.read_text()
        return source.split('# KF_' + name + '_BEGIN', 1)[1].split('# KF_' + name + '_END', 1)[0]

    def host_guard(self, reason):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            helper = directory / 'python3'
            helper.write_text('#!/bin/sh\nprintf "DRIVER_MEDIA_DRIFT reason=' + reason + '\\n"\nexit 3\n')
            helper.chmod(0o700)
            body = self.marked(HERE.parents[1] / 'scripts/bench/provision_host_driver.sh', 'DRIVER_PIN')
            result = subprocess.run(['bash', '-c', body + '\necho HARDWARE_TOUCHED'], env={**os.environ, 'PATH':str(directory)+':'+os.environ['PATH'], 'KF_RELEASE_DRIVER_PINS':'synthetic-fixture', 'HOST_DRIVER':'580.65.06', 'RUN':str(directory/'installer')}, capture_output=True, text=True)
            self.assertEqual(result.returncode,3,result.stdout+result.stderr)
            self.assertNotIn('HARDWARE_TOUCHED',result.stdout)
            self.assertIn('DRIVER_MEDIA_DRIFT reason='+reason,result.stdout)

    def provision_guard(self, reason):
        with tempfile.TemporaryDirectory() as root:
            log=Path(root)/'driver.log';log.write_text('DRIVER_MEDIA_DRIFT reason='+reason+'\n')
            body=self.marked(HERE.parents[1]/'scripts/bench/box/provision_full.sh','DRIVER_MEDIA_GATE').replace('/root/prov/driver.log',str(log))
            result=subprocess.run(['bash','-c',body+'\necho EXPENSIVE_BUILD_STARTED'],capture_output=True,text=True)
            self.assertEqual(result.returncode,3,result.stdout+result.stderr)
            self.assertNotIn('EXPENSIVE_BUILD_STARTED',result.stdout)
            self.assertIn('DRIVER_MEDIA_DRIFT reason='+reason,result.stdout)

    def test_host_stops_on_404_before_hardware(self):self.host_guard('HTTP_404')
    def test_host_stops_on_hash_before_hardware(self):self.host_guard('HASH')
    def test_provision_stops_on_404_before_builds(self):self.provision_guard('HTTP_404')
    def test_provision_stops_on_hash_before_builds(self):self.provision_guard('HASH')

if __name__=='__main__':unittest.main()
