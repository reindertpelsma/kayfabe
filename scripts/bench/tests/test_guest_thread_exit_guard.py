#!/usr/bin/env python3
"""Real GPU-free reproduction: leader exits, owned worker retains the image.

FALSIFIER: the leader's exe stays readable, or Popen reports whole-child exit
while a worker is blocked on a pipe. No sleeps or GPU participate.
"""
import contextlib
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('thread_guard', ROOT / 'scripts/bench/native_xid_guard.py')
G = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(G)
C_SOURCE = r'''
#include <pthread.h>
#include <unistd.h>
#include <stdlib.h>
static int worker_fd;
static void *worker(void *unused) { char c; (void)unused; read(worker_fd,&c,1); return 0; }
int main(int argc,char **argv) {
    pthread_t t; char c; (void)argc;
    worker_fd=atoi(argv[2]);
    if(pthread_create(&t,0,worker,0)) return 2;
    write(1,"R",1); read(atoi(argv[1]),&c,1); pthread_exit(0);
}
'''


class ThreadLeaderExitTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.folder = tempfile.TemporaryDirectory(prefix='s2-thread-exit-')
        cls.binary = Path(cls.folder.name) / 'owned-child'
        source = cls.binary.with_suffix('.c'); source.write_text(C_SOURCE)
        subprocess.run(['nice', '-n', '19', 'cc', '-pthread', str(source), '-o', str(cls.binary)], check=True)
        cls.revision = subprocess.check_output(['git','rev-parse','HEAD'], cwd=ROOT, text=True).strip()

    @classmethod
    def tearDownClass(cls): cls.folder.cleanup()

    def verify_exiting_leader(self, arm, wrong_worker=False, wrong_starttime=False, missing_live_exe=False):
        main_read, main_write = os.pipe(); worker_read, worker_write = os.pipe()
        child = subprocess.Popen([str(self.binary), str(main_read), str(worker_read)],
                                 pass_fds=(main_read, worker_read), stdout=subprocess.PIPE)
        try:
            self.assertEqual(child.stdout.read(1), b'R')
            context = G.capture_guest(child.pid, self.binary, self.revision, arm, ROOT)
            if not missing_live_exe:
                os.write(main_write,b'X')
                for _ in range(10000):
                    fields = Path('/proc',str(child.pid),'stat').read_text().rpartition(')')[2].split()
                    if fields[0]=='Z': break
                self.assertEqual(fields[0], 'Z', 'kernel event: leader exited while worker still blocked')
                with self.assertRaises(FileNotFoundError): Path('/proc',str(child.pid),'exe').resolve(strict=True)
            self.assertIsNone(child.poll(), 'the owned process still has a live worker')
            if wrong_worker: context['exe']='/different/executable'
            if wrong_starttime: context['starttime'] += 1
            refusal = None
            patch = mock.patch.object(Path, 'resolve', side_effect=FileNotFoundError('missing while live')) if missing_live_exe else contextlib.nullcontext()
            with patch:
                try: G.verify_guest_identity(context)
                except (G.GuardRefused, OSError) as error: refusal=error
            if wrong_worker or wrong_starttime or missing_live_exe:
                self.assertIsNotNone(refusal, 'live worker executable/starttime must still match the captured child')
            else:
                self.assertIsNone(refusal, 'live worker retains the exact captured executable after leader exit')
        finally:
            os.write(main_write,b'X')
            for fd in (main_read,main_write,worker_read,worker_write): os.close(fd)
            child.wait(timeout=5); child.stdout.close()

    def test_defer_leader_exit_preserves_exact_live_worker(self):
        self.verify_exiting_leader('--defer-liveness')

    def test_missing_leader_exit_preserves_exact_live_worker(self):
        self.verify_exiting_leader('--missing-page-fault')

    def test_defer_wrong_worker_executable_still_fatal(self):
        self.verify_exiting_leader('--defer-liveness', wrong_worker=True)

    def test_missing_wrong_worker_executable_still_fatal(self):
        self.verify_exiting_leader('--missing-page-fault', wrong_worker=True)

    def test_defer_changed_leader_starttime_still_fatal(self):
        self.verify_exiting_leader('--defer-liveness', wrong_starttime=True)

    def test_missing_changed_leader_starttime_still_fatal(self):
        self.verify_exiting_leader('--missing-page-fault', wrong_starttime=True)

    def test_defer_live_missing_executable_still_fatal(self):
        self.verify_exiting_leader('--defer-liveness', missing_live_exe=True)

    def test_missing_live_missing_executable_still_fatal(self):
        self.verify_exiting_leader('--missing-page-fault', missing_live_exe=True)


if __name__ == '__main__': unittest.main()
