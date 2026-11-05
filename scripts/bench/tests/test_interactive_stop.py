#!/usr/bin/env python3
"""Real owned CPU processes exercise shutdown; no VM, GPU, or global process search."""
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

HELPER = Path(__file__).resolve().parents[1] / 'display/owned_qemu.py'


class Stop(unittest.TestCase):
    def run_fixture(self, mode):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            monitor = root / 'qemu.mon'
            receipt = root / 'owned.json'
            child_code = '''
import signal,sys,time
from pathlib import Path
r=Path(sys.argv[1])
def term(sig,frame):
 (r/'term').touch();time.sleep(3);(r/'cleanup').touch();raise SystemExit(0)
signal.signal(signal.SIGTERM,term)
(r/'ready').touch()
while not (r/'acpi').exists(): time.sleep(.01)
(r/'cleanup').touch()
'''
            child = subprocess.Popen([sys.executable, '-c', child_code, str(root),
                '-name', 'kayfabe-interactive', '-monitor', f'unix:{monitor},server,nowait'])
            server = None
            try:
                end = time.monotonic() + 5
                while not (root / 'ready').exists() and time.monotonic() < end:
                    time.sleep(.01)
                self.assertTrue((root / 'ready').exists())
                record = subprocess.run([sys.executable, str(HELPER), 'record', str(child.pid),
                    sys.executable, str(monitor), str(receipt)], capture_output=True, timeout=5)
                self.assertEqual(record.returncode, 0, record.stderr)
                if mode == 'mismatch':
                    data = json.loads(receipt.read_text())
                    data['start_ticks'] += 1
                    receipt.write_text(json.dumps(data))
                if mode == 'acpi':
                    server = socket.socket(socket.AF_UNIX)
                    server.bind(str(monitor)); server.listen(1); server.settimeout(5)
                    def answer():
                        conn, _ = server.accept()
                        with conn:
                            (root / 'command').write_bytes(conn.recv(100))
                        (root / 'acpi').touch()
                    thread = threading.Thread(target=answer)
                    thread.start()
                if mode == 'launcher':
                    script = Path(os.environ.get('INTERACTIVE_FIXTURE_SCRIPT',
                        str(HELPER.with_name('interactive.sh')))).read_text()
                    cleanup = 'cleanup() {' + script.split('cleanup() {', 1)[1].split(
                        "\ntrap 'rc=130", 1)[0]
                    stop_function = ''
                    if 'stop_qemu() {' in script:
                        stop_function = 'stop_qemu() {' + script.split('stop_qemu() {', 1)[1].split('\n}', 1)[0] + '\n}\n'
                    env = os.environ.copy()
                    env.update(RECEIPT=str(receipt), PIDF=str(root/'pid'), RUN=str(root),
                        QSTOP=str(HELPER), OWN_BROKER='0', q=str(child.pid))
                    result = subprocess.run(['bash', '-c', '''
python3() { command python3 "$@" --acpi-seconds 0 --term-seconds 5; }
''' + stop_function + cleanup + '\ncleanup'], env=env,
                        capture_output=True, timeout=10)
                else:
                    result = subprocess.run([sys.executable, str(HELPER), 'stop', str(receipt),
                        '--acpi-seconds', '1' if mode == 'acpi' else '0',
                        '--term-seconds', '5'], capture_output=True, timeout=10)
                if mode == 'mismatch':
                    self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                    self.assertIsNone(child.poll())
                    self.assertFalse((root / 'term').exists())
                else:
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    child.wait(timeout=2)
                    self.assertTrue((root / 'cleanup').exists())
                    self.assertEqual((root / 'term').exists(), mode in ('term', 'launcher'))
                if mode == 'acpi':
                    thread.join(timeout=5)
                    self.assertEqual((root / 'command').read_bytes(), b'system_powerdown\n')
            finally:
                if server is not None:
                    server.close()
                if child.poll() is None:
                    os.kill(child.pid, signal.SIGTERM)
                child.wait(timeout=6)

    def test_term_allows_cleanup_longer_than_the_old_two_seconds(self):
        self.run_fixture('term')

    def test_acpi_shutdown_precedes_term(self):
        self.run_fixture('acpi')

    def test_reused_pid_identity_is_refused_without_a_signal(self):
        self.run_fixture('mismatch')

    def test_launcher_cleanup_allows_the_owned_process_to_finish(self):
        self.run_fixture('launcher')


if __name__ == '__main__':
    unittest.main()
