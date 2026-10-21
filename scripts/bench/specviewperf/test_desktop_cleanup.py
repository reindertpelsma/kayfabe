"""Owner override must never expand into permission to kill compute users."""
import unittest
from vfio_session import cleanup_names


class DesktopCleanupTests(unittest.TestCase):
    def test_known_desktop_names_allowed(self):
        self.assertEqual(cleanup_names({101: 'gnome-shell', 102: 'Xwayland', 103: 'firefox'}),
                         ['Xwayland', 'firefox', 'gnome-shell'])

    def test_monitoring_and_broker_allowed(self):
        self.assertEqual(cleanup_names({3: 'nvidia-smi', 4: 'nvkvm-display-b'}),
                         ['nvidia-smi', 'nvkvm-display-b'])

    def test_qemu_is_protected(self):
        with self.assertRaisesRegex(RuntimeError, 'PROTECTED_GPU_USER'):
            cleanup_names({55: 'qemu-system-x86'})

    def test_cuda_python_is_protected(self):
        with self.assertRaisesRegex(RuntimeError, 'PROTECTED_GPU_USER'):
            cleanup_names({55: 'python3', 56: 'gnome-shell'})

    def test_unknown_gpu_holder_is_protected(self):
        with self.assertRaisesRegex(RuntimeError, 'PROTECTED_GPU_USER'):
            cleanup_names({55: 'custom-compute'})

    def test_pid_one_is_never_a_kill_target(self):
        self.assertEqual(cleanup_names({1: 'systemd', 2135: 'systemd-logind'}), [])

    def test_pid_one_cannot_hide_unknown_name(self):
        with self.assertRaisesRegex(RuntimeError, 'PROTECTED_GPU_USER'):
            cleanup_names({1: 'custom-compute'})

    def test_logind_has_service_cleanup_not_pkill(self):
        self.assertNotIn('systemd-logind', cleanup_names({2135: 'systemd-logind', 3: 'gnome-shell'}))


if __name__ == '__main__':
    unittest.main()
