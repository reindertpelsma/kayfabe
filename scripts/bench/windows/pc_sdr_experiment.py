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
    # ⚠ 2026-10-07: the software-runlist host-owned scheduling experiment (option (b), AWAITING OWNER
    # CONFIRMATION; `kf_rm::sw_runlist_host`) is opt-in per run and never part of the default list.
    sw_runlist = '--sw-runlist-host-owned' in sys.argv
    if sw_runlist:
        sys.argv.remove('--sw-runlist-host-owned')
    # ★ 2026-10-08 (OWNER_RULINGS §U): the deferred-API trigger on Translated channels
    # (`KF3_DEFERRED_API`, default off until a Windows run shows it) — opt-in per run.
    deferred_api = '--deferred-api' in sys.argv
    if deferred_api:
        sys.argv.remove('--deferred-api')
    # ★ 2026-10-08 (run51): `--bar1-mb N` rewrites ONLY the pinned template's `bar1-size=134217728` token
    # (the Windows harness's 128 MiB, win_vm.sh) to N MiB. An experiment on the device's own property
    # (kf3.c default 256 MiB; V3_P4_PORT_MAP 2.3(d): derive from the host's BAR1); command.json records
    # the real argv. Opt-in, never in a default list.
    bar1_mb = None
    if '--bar1-mb' in sys.argv:
        i = sys.argv.index('--bar1-mb')
        bar1_mb = int(sys.argv[i + 1])
        del sys.argv[i:i + 2]
    path = Path('/var/lib/kf-windows-20261005/boundary-tools/pc_boundary_experiment.py')
    expected = 'dea3b1a1693b9a60b9514b7cc3cc4f89047469008227f3c35b87ad7096973944'
    if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
        raise SystemExit('Audited Windows runner changed')
    spec = importlib.util.spec_from_file_location('sdr_boundary', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    # Its existing manifest records every flag and its supervisor owns teardown.
    module.FLAGS = tuple(f for f in module.FLAGS if f != 'KF3_DISPLAY_TMO_CONSTRUCTOR_PROBE') + (
        'KF3_DISPLAY_SDR_COLOR', 'KF3_DISPLAY_METHOD_TRACE', 'KF3_KERNEL_GR_CE', 'KF3_KERNEL_NVDEC_CTX', 'KF3_KERNEL_NVENC_CTX', 'KF3_KERNEL_OFA_CTX', 'KF3_KERNEL_GR_WORK', 'KF3_SW_SUBCH_INERT', 'KF3_TRANSLATED_CE_RELAY', 'KF3_BAR0_TRACE', 'KF3_MAPLOG') + (
        ('KF3_SW_RUNLIST_HOST_OWNED',) if sw_runlist else ()) + (
        ('KF3_DEFERRED_API',) if deferred_api else ())
    if bar1_mb is not None:
        import json as _json

        class _Json:
            def __getattr__(self, name):
                return getattr(_json, name)

            def loads(self, text, *a, **k):
                doc = _json.loads(text, *a, **k)
                if isinstance(doc, dict) and 'argv' in doc:
                    doc['argv'] = [x.replace('bar1-size=134217728', f'bar1-size={bar1_mb * 1024 * 1024}') for x in doc['argv']]
                return doc

        shim = _Json()
        module.json = shim
    module.main()


if __name__ == '__main__':
    main()
