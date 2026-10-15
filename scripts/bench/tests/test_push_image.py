#!/usr/bin/env python3
"""An interrupted image stream must never leave a completed-chunk marker."""
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'diesweep' / 'push_image.sh'


class PushImageTests(unittest.TestCase):
    def run_failed_stream(self, failure):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'done').mkdir()
            body = SCRIPT.read_text().split('one() {', 1)[1].split('export -f one', 1)[0]
            body = ('one() {' + body).replace('/root/win', str(root))
            stub = r'''
BOX=box HOST=host SRC=/trusted-image
sleep() { :; }
zstd() { cat >/dev/null; [ "$FAILURE" != receiver ]; }
dd() { cat >/dev/null; }
ssh() {
  local command=${@: -1}
  case "$command" in
    *"[ -e"*) return 1 ;;
    *"nice -n 19"*) printf 'compressed stream'; [ "$FAILURE" != sender ] ;;
    *) bash -c "$command" ;;
  esac
}
export -f zstd dd ssh sleep
'''
            result = subprocess.run(['bash', '-c', 'set -o pipefail\n' + stub + body + '\none 0',
                                     'bash'], env={'PATH': '/usr/bin:/bin', 'FAILURE': failure},
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse((root / 'done' / 'c00').exists(), result.stdout + result.stderr)

    def test_failed_decompression_is_not_hidden_by_successful_dd(self):
        self.run_failed_stream('receiver')

    def test_sender_failure_does_not_mark_a_successful_receiver_complete(self):
        self.run_failed_stream('sender')


if __name__ == '__main__':
    unittest.main()
