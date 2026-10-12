#!/usr/bin/env python3
"""Pinned native matrix with read-only guest USERD sampling during its workload."""
import importlib.util
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('vfio_adapter', HERE.parent/'vfio-ring-p0/winapps_vfio.py')
ref = importlib.util.module_from_spec(spec); spec.loader.exec_module(ref)

class CursorVm(ref.VfioVm):
    sampler = None
    sampler_log = None

    def launch(self, tag):
        super().launch(tag)
        boot = Path(self.run_dir())
        self.sampler_log = (boot/'userd-sampler.log').open('w')
        self.sampler = subprocess.Popen([sys.executable, '-I', str(HERE/'sample_userd.py'), str(boot), str(boot/'userd-samples.jsonl')], stdout=self.sampler_log, stderr=subprocess.STDOUT)
        self.log(f'USERD sampler launched pid={self.sampler.pid}; passive, result checked separately')

    def stop(self):
        if self.sampler:
            (Path(self.run_dir())/'stop-userd-sampler').touch()
            try:
                rc = self.sampler.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.sampler.terminate()
                rc = self.sampler.wait(timeout=10)
            self.log(f'USERD sampler exit={rc} (not an app verdict)')
            self.sampler_log.close()
            self.sampler = None
        return super().stop()

ref.winapps.BrokerVm = CursorVm
if __name__ == '__main__': raise SystemExit(ref.winapps.main())
