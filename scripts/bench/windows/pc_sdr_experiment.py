#!/usr/bin/env python3
"""Run the audited fresh-overlay Windows harness with real GPU colour.
Constructor-only display probes stay disabled; the named product revision
controls the implemented colour stages. Uses its ordinary CLI.
Only the Kayfabe arm is allowed; this never detaches the physical host display.
"""
import hashlib
import importlib.util
from pathlib import Path
import sys


def main():
    if '--arm' not in sys.argv or sys.argv[sys.argv.index('--arm') + 1] != 'kayfabe':
        raise SystemExit('Require --arm kayfabe')
    if any(x in sys.argv for x in ('--ilut-probe', '--tmo-surface-probe', '--olut-probe')):
        raise SystemExit('Constructor-only probes cannot be combined with real colour')
    path = Path('/var/lib/kf-windows-20261005/boundary-tools/pc_boundary_experiment.py')
    expected = 'dea3b1a1693b9a60b9514b7cc3cc4f89047469008227f3c35b87ad7096973944'
    if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
        raise SystemExit('Audited Windows runner changed')
    spec = importlib.util.spec_from_file_location('sdr_boundary', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    # Its existing manifest records every flag and its supervisor owns teardown.
    module.FLAGS = tuple(f for f in module.FLAGS if f != 'KF3_DISPLAY_TMO_CONSTRUCTOR_PROBE') + (
        'KF3_DISPLAY_SDR_COLOR', 'KF3_DISPLAY_METHOD_TRACE', 'KF3_KERNEL_GR_CE', 'KF3_KERNEL_NVDEC_CTX', 'KF3_KERNEL_NVENC_CTX', 'KF3_KERNEL_OFA_CTX', 'KF3_KERNEL_GR_WORK', 'KF3_SW_SUBCH_INERT', 'KF3_TRANSLATED_CE_RELAY', 'KF3_BAR0_TRACE', 'KF3_MAPLOG')
    module.main()


if __name__ == '__main__':
    main()
