"""GPU-free contracts for the LX1 host harness; no hardware or sysfs writes."""
import importlib.util
import pathlib
import unittest

MODULE = pathlib.Path(__file__).with_name('host.py')


def harness():
    if not MODULE.is_file():
        raise AssertionError('LX1 host harness is not implemented (new API, not a product defect)')
    spec = importlib.util.spec_from_file_location('lx1_host', MODULE)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class HostContracts(unittest.TestCase):
    def setUp(self):
        self.h = harness()
        self.c = dict(root='/data/codex-LX1', disk='/data/codex-LX1/guest.qcow2',
                      run='test-01', driver='595.91.07',
                      bins='/benchmark/bins', pci=['0000:01:00.0', '0000:01:00.1'])

    def test_a_removes_inherited_diagnostic(self):
        _, env = self.h.plan(self.c, 'A', {'PATH': '/bin', 'KF3_DEBUG': '1'})
        self.assertNotIn('KF3_DEBUG', env)

    def test_a_has_zero_kf3_variables(self):
        _, env = self.h.plan(self.c, 'A', {'KF3_TSG_MIRROR': '1'})
        self.assertEqual({}, {k: v for k, v in env.items() if k.startswith('KF3_')})

    def test_b_explicit_mirror(self):
        _, env = self.h.plan(self.c, 'B', {'KF3_DEBUG': '1'})
        self.assertEqual({'KF3_TSG_MIRROR': '1'}, {k: v for k, v in env.items() if k.startswith('KF3_')})

    def test_d_explicit_kill_switch(self):
        _, env = self.h.plan(self.c, 'D', {})
        self.assertEqual('0', env['KF3_TSG_MIRROR'])

    def test_same_disk_all_arms(self):
        for arm in 'ABCDP':
            args, _ = self.h.plan(self.c, arm, {})
            self.assertIn('file=' + self.c['disk'] + ',if=none,id=disk0,format=qcow2,cache=none,aio=threads', args)

    def test_same_cpu_memory_all_arms(self):
        for arm in 'ABCDP':
            args, _ = self.h.plan(self.c, arm, {})
            self.assertIn('host,-vmx,migratable=off,+invtsc', args)
            self.assertIn('memory-backend-memfd,id=ram0,size=12288M,share=on', args)
            self.assertEqual('8', args[args.index('-smp') + 1])

    def test_manifest_disk_used_in_vfio(self):
        args, _ = self.h.plan(self.c, 'C', {})
        disks = [x for x in args if x.startswith('file=') and 'id=disk0' in x]
        self.assertEqual(1, len(disks))
        self.assertTrue(disks[0].startswith('file=' + self.c['disk'] + ','))

    def test_shared_ram_required_by_kayfabe(self):
        args, _ = self.h.plan(self.c, 'B', {})
        self.assertIn('memory-backend-memfd,id=ram0,size=12288M,share=on', args)

    def test_a_c_same_binary(self):
        self.assertEqual(self.h.plan(self.c, 'A', {})[0][0], self.h.plan(self.c, 'C', {})[0][0])

    def test_c_contains_both_functions(self):
        args, _ = self.h.plan(self.c, 'C', {})
        for dev in self.c['pci']:
            self.assertIn('vfio-pci,host=' + dev, args)
        self.assertFalse(any('kf3-gpu' in x for x in args))

    def test_refuses_source_disk(self):
        self.c['disk'] = '/other/seed.qcow2'
        with self.assertRaises(ValueError):
            self.h.plan(self.c, 'A', {})

    def test_refuses_traversal_run(self):
        self.c['run'] = '../other'
        with self.assertRaises(ValueError):
            self.h.plan(self.c, 'C', {})

    def test_refuses_outside_root(self):
        self.c['root'] = '/data/someone-else'
        with self.assertRaises(ValueError):
            self.h.plan(self.c, 'P', {})

    def test_refuses_qemu(self):
        with self.assertRaises(RuntimeError):
            self.h.idle_guard(['qemu'], [], [])

    def test_refuses_broker(self):
        with self.assertRaises(RuntimeError):
            self.h.idle_guard([], ['broker'], [])

    def test_refuses_gpu_client(self):
        with self.assertRaises(RuntimeError):
            self.h.idle_guard([], [], ['client'])

    def test_idle_accepts_empty(self):
        self.h.idle_guard([], [], [])

    def test_refuses_extra_iommu_member(self):
        with self.assertRaises(RuntimeError):
            self.h.group_guard(self.c['pci'], self.c['pci'] + ['0000:02:00.0'], True)

    def test_refuses_missing_iommu_member(self):
        with self.assertRaises(RuntimeError):
            self.h.group_guard(self.c['pci'], [self.c['pci'][0]], True)

    def test_refuses_no_amd_desktop(self):
        with self.assertRaises(RuntimeError):
            self.h.group_guard(self.c['pci'], self.c['pci'], False)

    def test_group_exact_accepts(self):
        self.h.group_guard(self.c['pci'], list(reversed(self.c['pci'])), True)

    def test_rebind_refuses_live_vm(self):
        with self.assertRaises(RuntimeError):
            self.h.restore_guard(True, {'0000:01:00.0': 'nvidia'})

    def test_rebind_requires_original_drivers(self):
        with self.assertRaises(RuntimeError):
            self.h.restore_guard(False, {})

    def test_rebind_rejects_unrecorded_driver(self):
        with self.assertRaises(RuntimeError):
            self.h.restore_guard(False, {'0000:01:00.0': 'unknown'})

    def test_rebind_accepts_record(self):
        self.h.restore_guard(False, {'0000:01:00.0': 'nvidia', '0000:01:00.1': 'snd_hda_intel'})

    def test_lease_pid_reuse_refused(self):
        with self.assertRaises(RuntimeError):
            self.h.lease_guard({'pid': 30, 'start': '100'}, 30, '101', {30})

    def test_lease_requires_flock_holder(self):
        with self.assertRaises(RuntimeError):
            self.h.lease_guard({'pid': 30, 'start': '100'}, 30, '100', {40})

    def test_lease_accepts_matching_holder(self):
        self.h.lease_guard({'pid': 30, 'start': '100'}, 30, '100', {30})

    def test_sampler_fields_and_missing_nvml_are_explicit(self):
        from unittest.mock import patch
        import subprocess
        def query(argv, **kwargs):
            return subprocess.CompletedProcess(argv, 9, '', 'GPU is owned by VFIO')
        with patch.object(self.h.subprocess, 'run', query):
            row = self.h.snapshot()
        self.assertEqual(9, row['nvml']['rc'])
        self.assertEqual(9, row['nvml_detail']['rc'])
        self.assertGreaterEqual(row['sample_end_ns'], row['realtime_ns'])
        self.assertGreater(row['monotonic_ns'], 0)
        self.assertEqual({'cpu', 'memory', 'io'}, set(row['pressure']))

    def test_sampler_vcpu_identity_with_fake_proc(self):
        from unittest.mock import patch
        import subprocess
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            task = pathlib.Path(tmp) / 'proc/123/task/124'
            task.mkdir(parents=True)
            fields = ['S'] + [str(i) for i in range(4, 53)]
            (task / 'stat').write_text('124 (CPU 0/KVM) ' + ' '.join(fields))
            (task / 'comm').write_text('CPU 0/KVM')
            (task / 'schedstat').write_text('1000 2000 30')
            original_path = pathlib.Path
            def path(value):
                if str(value).startswith('/proc/123/'):
                    return original_path(tmp) / str(value).lstrip('/')
                return original_path(value)
            with patch.object(self.h, 'Path', path), patch.object(self.h, 'proc_start', return_value='900'), patch.object(self.h.subprocess, 'run', side_effect=lambda argv, **kw: subprocess.CompletedProcess(argv, 0, 'sample', '')):
                row = self.h.snapshot(123)
        self.assertEqual({'pid': 123, 'start': '900'}, row['qemu'])
        self.assertEqual([{'tid': 124, 'name': 'CPU 0/KVM', 'start': '22', 'schedstat': '1000 2000 30'}], row['vcpu'])

    def test_proc_stat_with_spaces_in_name(self):
        fields = ['S'] + [str(i) for i in range(4, 53)]
        self.assertEqual('22', self.h.starttime('123 (CPU 0/KVM) ' + ' '.join(fields)))

    def test_proc_stat_with_parentheses_in_name(self):
        fields = ['S'] + [str(i) for i in range(4, 53)]
        self.assertEqual('22', self.h.starttime('123 (odd (name)) ' + ' '.join(fields)))


if __name__ == '__main__':
    unittest.main()
