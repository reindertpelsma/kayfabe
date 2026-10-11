#!/usr/bin/env python3
"""Same pinned app matrix, reference VFIO launcher instead of kayfabe."""
import os
from pathlib import Path
import subprocess
import sys

BENCH = Path('/data/kf-managed-host/scripts/bench/windows/appmatrix')
sys.path.insert(0, str(BENCH))
import winapps


class VfioVm(winapps.BrokerVm):
    def __init__(self, broker, w_dir, kf3_rev, run_base, log, **kwargs):
        super().__init__(broker, w_dir, kf3_rev, run_base, log, **kwargs)
        self.reference_dir = Path(os.environ['VDR_DIR'])

    def run_dir(self):
        return str(self.reference_dir / f'boot{self.n}')

    def launch(self, tag):
        self.n = self.base + self.count
        self.count += 1
        # Exclusive session owns detach/restore outside the unchanged matrix.
        command = [self.broker, 'run', str(self.n), 'fresh' if self.count == 1 else 'reuse']
        reply = subprocess.run(['bash', *command], capture_output=True, text=True, timeout=300)
        self.log(f'VFIO reference launch boot={self.n} rc={reply.returncode}')
        if reply.returncode:
            raise RuntimeError('VFIO launch failed: ' + reply.stderr[-500:])
        self.qpid = int((Path(self.run_dir()) / 'qemu.pid').read_text())

    def stop(self):
        if self.n is None:
            return 'none'
        reply = subprocess.run(['bash', self.broker, 'stop', str(self.n)], capture_output=True, text=True, timeout=600)
        how = 'clean' if reply.returncode == 0 else f'vfio-stop-rc{reply.returncode}'
        if self.alive():
            self._qmp().cmd('quit')
            how += '+qmp-quit'
        self.log('VFIO reference stopped ' + how)
        return how

    def gsp_cycles(self):
        # Observer is present, but matrix's kayfabe phase parser does not apply.
        # Retain JSONL for a separate observer-schema-aware count; never claim 0.
        return None


winapps.BrokerVm = VfioVm
if __name__ == '__main__':
    raise SystemExit(winapps.main())
