#!/usr/bin/env python3
"""Tier-3 evidence collector invoked by the existing driver sweep, before guests.

HYPOTHESIS: exact native controls plus engine-table queries expose the missing
CLIENT relation. FALSIFIER: a refused/no-CLIENT table or unmatched native fault.
An observation is never independent runtime derivation: this stage returns 3
when collection completes, so sweep.sh cannot start a guest from calibration.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

SPEC = importlib.util.spec_from_file_location('native_route_measurement', Path(__file__).with_name('native_route_measurement.py'))
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


def execute(name, argv, env, dest):
    print('NATIVE_COMMAND ' + json.dumps({'name': name, 'argv': [str(a) for a in argv]}), flush=True)
    with dest.open('w') as stream:
        child = subprocess.Popen(argv, env=env, stdout=stream, stderr=subprocess.STDOUT)
        try:
            rc = child.wait(timeout=180)
        except BaseException:
            child.terminate()
            child.wait(timeout=15)
            raise
    print(f'NATIVE_COMMAND_EXIT name={name} pid={child.pid} rc={rc}', flush=True)
    if rc:
        raise RuntimeError(f'{name} rc={rc}')
    return child.pid


def run(host, arch, repo, out, ladder, probe, execute=execute):
    out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, KF_PROBE_I_KNOW='1', KF_PROBE_EXTRA_ONLY='1', KF_PROBE_HEAD='3212')
    def command(name, argv, settings=env):
        return execute(name, [str(x) for x in argv], settings, out / (name + '.log'))
    command('identity', ['nvidia-smi', '--query-gpu=name,driver_version', '--format=csv,noheader'])
    identity = (out / 'identity.log').read_text().strip().splitlines()
    if len(identity) != 1 or identity[0].rsplit(',', 1)[-1].strip() != host:
        raise ValueError('host driver mismatch or multiple GPUs')
    with tempfile.TemporaryDirectory(prefix='kf-native-proof-') as tmp:
        kit = Path(tmp); kit.chmod(0o755)
        public_probe = kit / 'kf-ctl-probe'
        shutil.copy2(probe, public_probe); public_probe.chmod(0o755)
        query = kit / 'engine.tsv'
        query.write_text('subdevice\t0x20801112\t3212\n'); query.chmod(0o644)
        command('nobody_table', ['runuser', '-u', 'nobody', '--', 'env', 'KF_PROBE_I_KNOW=1',
                                'KF_PROBE_EXTRA_ONLY=1', 'KF_PROBE_HEAD=3212', public_probe, query])
        command('root_table', [public_probe, query], dict(env, KF_PROBE_ALLOW_ADMIN='1'))
    die = re.search(r'^PROBE_DIE arch=(\S+) impl=(\S+) die=(\S+)$', (out / 'nobody_table.log').read_text(), re.M)
    if die is None or die[3] != arch:
        raise ValueError('host implementation differs from sweep architecture')
    (out / 'die.json').write_text(json.dumps({'kind': 'MEASURED', 'die': die[3], 'arch': die[1],
                                            'impl': die[2], 'driver': host}, indent=2) + '\n')
    command('pce_mask', [ladder, '--pce-mask-probe'])
    journal = ['journalctl', '-k', '-b', '-o', 'short-iso-precise', '--no-pager']
    command('before', journal)
    before = M.G.bounded(out / 'before.log')
    if M.G.xid_lines(before):
        raise ValueError('preexisting Xid; no new intentional faults permitted')
    pids = {}
    for name, flag in [('defer', '--defer-liveness'), ('missing', '--missing-page-fault')]:
        pids[name] = command(name, [ladder, flag])
        (out / 'pids.json').write_text(json.dumps(pids, indent=2) + '\n')
    command('after', journal)
    result = M.measure(before, M.G.bounded(out / 'after.log'), M.G.bounded(out / 'defer.log'),
                       M.G.bounded(out / 'missing.log'), pids)
    assert result['approved_for_guest'] is False
    (out / 'observation.json').write_text(json.dumps(result, indent=2) + '\n')
    print('NATIVE_PROOF_REFUSED: observation complete; independent runtime CLIENT derivation unresolved', flush=True)
    return 3


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('host', 'arch', 'repo', 'directory', 'ladder', 'probe'):
        parser.add_argument(name)
    args = parser.parse_args()
    if 'Open Kernel Module' not in Path('/proc/driver/nvidia/version').read_text():
        raise ValueError('open NVIDIA kernel module required')
    out = Path(args.directory) / 'native_proof'
    out.mkdir(parents=True, exist_ok=True)
    hashes = {name: hashlib.sha256(Path(path).read_bytes()).hexdigest()
              for name, path in [('ladder', args.ladder), ('ctl_probe', args.probe)]}
    (out / 'binary_hashes.json').write_text(json.dumps(hashes, indent=2) + '\n')
    return run(args.host, args.arch, Path(args.repo), out, Path(args.ladder), Path(args.probe))


if __name__ == '__main__':
    raise SystemExit(main())
