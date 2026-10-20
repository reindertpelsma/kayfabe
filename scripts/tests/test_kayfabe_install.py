#!/usr/bin/env python3
"""GPU-free contracts for the release launcher, doctor and prefix installer."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]

class InstallPath(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location('kayfabe', ROOT / 'scripts/kayfabe.py')
        cls.k = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.k)

    def facts(self):
        return dict(total_mb=12288, free_mb=11000, bar1_mb=256, kvm=True,
                    uefi_code='/firmware/OVMF_CODE.fd',uefi_vars_template='/firmware/OVMF_VARS.fd',
                    devices=True, udmabuf=True, driver='595.91.07', family='Ada', model='RTX 4070',
                    qemu_version='0.1.0-beta.1', broker_version='0.1.0-beta.1',
                    graphical=True, disk_free=8<<30, hugepages=0, memlock=8<<20,
                    memcg='max', groups=['kvm'], runtime='/run/user/1000', uid=1000)

    def options(self, *extra):
        return self.k.parser().parse_args(['--disk', '/tmp/disk with spaces.qcow2',
                                          '--guest-driver', '580.65.06', *extra])

    def test_small_bar_and_vram_follow_the_documented_budget(self):
        self.assertEqual(self.k.geometry(self.facts(), True), (8192, 128<<20, 32<<20))
        facts=self.facts();facts.update(total_mb=6144,bar1_mb=16384)
        self.assertEqual(self.k.geometry(facts, False), (4096, 8192<<20, 32<<20))

    def test_insufficient_resources_refuse_instead_of_guessing(self):
        for total,bar in [(2048,256),(12288,64)]:
            facts=self.facts();facts.update(total_mb=total,bar1_mb=bar)
            with self.assertRaises(ValueError):self.k.geometry(facts,True)

    def test_windows_needs_one_profile_and_no_environment_flags(self):
        command=self.k.qemu_command(self.options('--os','windows'),self.facts(),Path('/opt/kayfabe'))
        device=command[command.index('-device')+1]
        self.assertIn('guest-os=windows',device)
        self.assertIn('display=on',device)
        self.assertIn('gop=on',device)
        self.assertFalse(any('KF3_' in part for part in command))
        self.assertIn('/tmp/disk with spaces.qcow2',command[command.index('-drive')+1])
        self.assertTrue(any('memory-backend-memfd' in part for part in command))

    def test_linux_headless_omits_display_and_broker(self):
        command=self.k.qemu_command(self.options('--no-display'),self.facts(),Path('/opt/kayfabe'))
        device=command[command.index('-device')+1]
        self.assertIn('guest-os=linux',device)
        self.assertNotIn('display-broker=',device)
        self.assertNotIn('display=on',device)
        self.assertNotIn('gop=on',device)

    def test_explicit_resources_and_broker_are_preserved(self):
        command=self.k.qemu_command(self.options('--broker','/tmp/b.sock','--fb-mb','4096','--bar1-size','134217728'),self.facts(),Path('/opt/kayfabe'))
        device=command[command.index('-device')+1]
        for field in ['fb-mb=4096','bar1-size=134217728','display-broker=/tmp/b.sock','display-broker-uid=1000']:
            self.assertIn(field,device)

    def test_versions_must_match(self):
        facts=self.facts();facts['broker_version']='wrong'
        rows=self.k.doctor_rows(facts,'0.1.0-beta.1',True)
        self.assertTrue(any(row['status']=='FAIL' and row['check']=='versions' for row in rows))

    def test_missing_kvm_or_permissions_are_failures_with_fixes(self):
        facts=self.facts();facts.update(kvm=False,devices=False)
        rows=self.k.doctor_rows(facts,'0.1.0-beta.1',False)
        for check in ['kvm','permissions']:
            row=next(row for row in rows if row['check']==check)
            self.assertEqual(row['status'],'FAIL');self.assertTrue(row['fix'])

    def _display_requires_udmabuf(self, os_name):
        facts=self.facts();facts['udmabuf']=False
        options=self.options('--os',os_name,'--broker','/tmp/fixture.sock')
        self.assertFalse(options.no_display)
        rows=self.k.doctor_rows(facts,'0.1.0-beta.1',True)
        row=next(row for row in rows if row['check']=='udmabuf')
        self.assertEqual(row['status'],'FAIL')
        self.assertIn('/dev/udmabuf',row['detail'])
        self.assertIn('kvm',row['fix'])

    def test_windows_display_refuses_missing_udmabuf_access(self):
        self._display_requires_udmabuf('windows')

    def test_linux_broker_refuses_missing_udmabuf_access(self):
        self._display_requires_udmabuf('linux')

    def test_headless_does_not_require_udmabuf(self):
        facts=self.facts();facts['udmabuf']=False
        rows=self.k.doctor_rows(facts,'0.1.0-beta.1',False)
        self.assertFalse(any(row['check']=='udmabuf' and row['status']=='FAIL' for row in rows))

    def test_no_hugepages_or_small_memlock_does_not_invent_a_root_requirement(self):
        rows=self.k.doctor_rows(self.facts(),'0.1.0-beta.1',True)
        self.assertFalse(any(row['status']=='FAIL' and row['check'] in ['hugepages','memlock'] for row in rows))
        self.assertTrue(any(row['check']=='memcg' for row in rows))

    def test_install_is_private_and_uninstall_only_removes_its_manifest(self):
        with tempfile.TemporaryDirectory() as raw:
            root=Path(raw);source=root/'release';source.mkdir();(source/'VERSION').write_text('0.1.0-beta.1\n')
            (source/'bin').mkdir();(source/'bin/qemu-system-x86_64').write_text('fixture')
            prefix=root/'prefix';self.k.install_prefix(source,prefix)
            (prefix/'keep-user-file').write_text('keep')
            self.k.uninstall_prefix(prefix)
            self.assertTrue((prefix/'keep-user-file').exists())
            self.assertFalse((prefix/'bin/qemu-system-x86_64').exists())

    def test_install_refuses_existing_destination_and_dangerous_prefixes(self):
        with tempfile.TemporaryDirectory() as raw:
            root=Path(raw);source=root/'release';source.mkdir();prefix=root/'prefix';prefix.mkdir()
            with self.assertRaises(ValueError):self.k.install_prefix(source,prefix)
            for bad in ['/', '/usr', '/usr/bin', '/opt']:
                with self.assertRaises(ValueError):self.k.install_prefix(source,Path(bad))

    def test_derived_geometry_always_reserves_documented_headroom(self):
        for total,bar in [(6144,256),(8192,512),(12288,16384)]:
            facts=self.facts();facts.update(total_mb=total,bar1_mb=bar)
            for display in [False,True]:
                fb,b1,b2=self.k.geometry(facts,display)
                self.assertLessEqual(fb+2048,total)
                boot=1920*1080*4 if display else 0
                self.assertLessEqual(b1+b2+(1+16)*(1<<20)+boot,bar*(1<<20))

    def test_declared_os_and_driver_are_independent_device_properties(self):
        for os_name in ['windows','linux']:
            command=self.k.qemu_command(self.options('--os',os_name),self.facts(),Path('/opt/kayfabe'))
            device=command[command.index('-device')+1]
            self.assertIn('guest-os='+os_name,device)
            self.assertIn('guest-driver=580.65.06',device)

    def test_old_qemu_is_refused_even_with_the_current_broker(self):
        facts=self.facts();facts['qemu_version']='old'
        row=next(row for row in self.k.doctor_rows(facts,'0.1.0-beta.1',True) if row['check']=='versions')
        self.assertEqual(row['status'],'FAIL')

    def _print_has_no_launch_or_state_effect(self,os_name):
        import contextlib,io,json
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as raw:
            root=Path(raw);disk=root/'disk.qcow2';disk.write_bytes(b'fixture')
            facts=root/'facts.json';facts.write_text(json.dumps(self.facts()))
            with patch.object(self.k.subprocess,'run',side_effect=AssertionError('dry-run executed a process')), \
                 patch.object(self.k.subprocess,'Popen',side_effect=AssertionError('dry-run launched TPM')), \
                 patch.object(Path,'mkdir',side_effect=AssertionError('dry-run wrote state')), \
                 contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(self.k.main(['--os',os_name,'--disk',str(disk),'--guest-driver','580.65.06',
                                             '--facts',str(facts),'--print']),0)
            self.assertFalse((root/'.kayfabe').exists())

    def test_windows_print_never_starts_a_process_or_writes_state(self):
        self._print_has_no_launch_or_state_effect('windows')

    def test_linux_print_never_starts_a_process_or_writes_state(self):
        self._print_has_no_launch_or_state_effect('linux')

    def _reject_manifest_escape(self,absolute):
        import json,hashlib
        with tempfile.TemporaryDirectory() as raw:
            root=Path(raw);prefix=root/'prefix';prefix.mkdir();outside=root/'outside';outside.write_bytes(b'owned fixture')
            name=str(outside) if absolute else '../outside'
            (prefix/'.kayfabe-install.json').write_text(json.dumps({name:hashlib.sha256(outside.read_bytes()).hexdigest()}))
            with self.assertRaises(ValueError):self.k.uninstall_prefix(prefix)
            self.assertEqual(outside.read_bytes(),b'owned fixture')

    def test_uninstall_refuses_traversal(self):self._reject_manifest_escape(False)

    def test_uninstall_refuses_absolute_manifest_paths(self):self._reject_manifest_escape(True)

if __name__=='__main__':unittest.main()
