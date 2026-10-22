"""Safety-contract tests: no GPU or root access required."""
import unittest
from pathlib import Path
from vfio_session import make_argv, abort_reason


class PlanTests(unittest.TestCase):
    def setUp(self):
        self.root = Path('/data/codex-VF1')
        self.owner = ['qemu-system-x86_64', '-name', 'owner', '-m', '12288', '-smp', '8',
                      '-cpu', 'host,-vmx,+invtsc', '-drive', 'file=/owner/vars.fd,if=pflash,unit=1',
                      '-drive', 'file=/owner/windows.qcow2,if=none,id=disk0,cache=none,aio=threads',
                      '-qmp', 'unix:/owner/qmp.sock,server=on,wait=off',
                      '-serial', 'file:/owner/serial.log',
                      '-chardev', 'socket,id=qga0,path=/owner/qga.sock,server=on,wait=off',
                      '-device', 'kf3-gpu,id=kf0,addr=0x6', '-uuid', 'example-uuid']
        self.argv = make_argv(self.owner, self.root)

    def test_standalone_qemu_switches_are_supported(self):
        owner = self.owner + ['-no-shutdown']
        self.assertEqual(make_argv(owner, self.root)[-1], '-no-shutdown')

    def test_standalone_switch_keeps_following_options_aligned(self):
        owner = self.owner[:1] + ['-nodefaults'] + self.owner[1:]
        native = make_argv(owner, self.root)
        self.assertIn('-nodefaults', native)
        self.assertEqual(native[native.index('-m') + 1], '12288')
        self.assertIn('file=/data/codex-VF1/vfio-overlay.qcow2,if=none,id=disk0,cache=none,aio=threads', native)

    def test_no_kayfabe_device_in_native_reference(self):
        self.assertFalse(any('kf3-gpu' in a for a in self.argv))

    def test_both_physical_functions_attached(self):
        joined = ' '.join(self.argv)
        for bdf in ['0000:01:00.0', '0000:01:00.1']:
            self.assertIn('vfio-pci,host=' + bdf, joined)

    def test_never_writes_owners_disk(self):
        drives = [self.argv[i+1] for i, a in enumerate(self.argv) if a == '-drive']
        self.assertFalse(any('file=/owner/' in d for d in drives))
        self.assertIn('file=/data/codex-VF1/vfio-overlay.qcow2,if=none,id=disk0,cache=none,aio=threads', drives)

    def test_uefi_variables_are_private_copy(self):
        self.assertIn('file=/data/codex-VF1/OVMF_VARS.fd,if=pflash,unit=1', self.argv)

    def test_owner_cpu_ram_vcpus_identity_preserved(self):
        for option in ['-cpu', '-m', '-smp', '-uuid']:
            self.assertEqual(self.argv[self.argv.index(option)+1], self.owner[self.owner.index(option)+1])

    def test_all_control_endpoints_are_lane_owned(self):
        for option in ['-qmp', '-serial', '-chardev']:
            self.assertNotIn('/owner/', self.argv[self.argv.index(option)+1])


class AbortTests(unittest.TestCase):
    def check(self, **kwargs):
        values = dict(xid_before=26, xid_now=26, now=1000, deadline=10000,
                      progress_at=999, alive=True, stop=False)
        values.update(kwargs)
        return abort_reason(**values)

    def test_new_xid_aborts(self):
        self.assertEqual(self.check(xid_now=27), 'NEW_HOST_XID')

    def test_xid_takes_precedence_over_stop(self):
        self.assertEqual(self.check(xid_now=28, stop=True), 'NEW_HOST_XID')

    def test_deadline_aborts(self):
        self.assertEqual(self.check(now=10000), 'RESTORE_DEADLINE')

    def test_deadline_takes_precedence_over_live_progress(self):
        self.assertEqual(self.check(now=10001, progress_at=10001), 'RESTORE_DEADLINE')

    def test_fifteen_minutes_no_progress_aborts(self):
        self.assertEqual(self.check(now=1900, progress_at=1000), 'NO_PROGRESS_15_MIN')

    def test_overdue_progress_aborts(self):
        self.assertEqual(self.check(now=1901, progress_at=1000), 'NO_PROGRESS_15_MIN')

    def test_recent_progress_continues(self):
        self.assertIsNone(self.check(now=1899, progress_at=1000))

    def test_stop_restores(self):
        self.assertEqual(self.check(stop=True), 'REQUESTED_STOP')

    def test_dead_qemu_restores(self):
        self.assertEqual(self.check(alive=False), 'QEMU_EXIT')


if __name__ == '__main__':
    unittest.main()
