from pathlib import Path
import ast
import datetime
import hashlib
import json
import os
import signal
import subprocess
import tarfile
import time

D = Path(__file__).resolve().parent
saved_plan = D / 'parallel_eight_plan.json'
if saved_plan.exists():
    end = datetime.datetime.fromisoformat(
        json.loads(saved_plan.read_text())['original_deadline_utc'])
else:
    end = (datetime.datetime.now(datetime.timezone.utc)
           + datetime.timedelta(seconds=1200))
now = lambda: datetime.datetime.now(datetime.timezone.utc)
conf = D / 'ssh.conf'
alias = (D / 'alias').read_text().strip()
ssh = ['ssh', '-F', str(conf), alias]
m = json.loads((D / 'media_manifest.json').read_text())
plan = {
    'utc': now().isoformat(), 'own_id': int((D/'own-id').read_text()), 'workers_per_file': 8,
    'file_start_stagger_seconds': 3, 'helper_unmodified': True,
    'helper_sha256': hashlib.sha256((D / 'checked_chunks.py').read_bytes()).hexdigest(),
    'original_deadline_utc': end.isoformat(), 'total_media_upload_budget_seconds': 1200,
    'prior_lease_transport_learning': 'use tested8 streams; avoid16 authentication burst',
    'cause': 'inferred SSH unauthenticated-start burst; measured maxstartups10:30:100',
    'hypothesis': 'tested8-stream groups with staggered starts finish within original bound',
    'falsifier': 'any failed chunk/size/hash or deadline; no controller starts on failure',
}
(D / 'parallel_eight_plan.json').write_text(json.dumps(plan, indent=2) + '\n')
subprocess.run(ssh + ['test ! -e /root/prov/sweep_controller.pid'],
               check=True, timeout=20)
subprocess.run(ssh + ['install -d -m 0755 /workspace/bw-capc-validation/cache'],
               check=True, timeout=20)
children = []
try:
    for row in m['files']:
        if row['expected_bytes'] < 10000000:
            dest = '/workspace/bw-capc-validation/cache/' + row['file'] + '.part'
            with (D / 'media' / row['file']).open('rb') as src:
                subprocess.run(ssh + ['umask 077; cat > ' + dest], stdin=src,
                               timeout=120, check=True)
            continue
        dest = '/workspace/bw-capc-validation/cache/' + row['file'] + '.part'
        args = ['python3', str(D / 'checked_chunks.py'), str(D / 'media' / row['file']),
                str(conf), dest, str(row['expected_bytes']), row['expected_sha256'],
                '--workers', '8']
        out = (D / (row['file'] + '.parallel8.log')).open('w')
        p = subprocess.Popen(args, stdout=out, stderr=subprocess.STDOUT,
                             start_new_session=True)
        out.close()
        children.append((p, row['file']))
        print('CHECKED8_STARTED', row['file'], p.pid, flush=True)
        time.sleep(3)
    for p, name in children:
        remaining = int((end - now()).total_seconds())
        assert remaining > 0, 'original transfer deadline reached'
        assert p.wait(timeout=remaining) == 0, 'checked transport failed: ' + name
        print('CHECKED8_PIN_OK', name, flush=True)
except BaseException:
    for p, _ in children:
        if p.poll() is None:
            os.killpg(p.pid, signal.SIGTERM)
            p.wait(timeout=15)
    raise
module = ast.parse((D / 'cache_upload.py').read_text())
verify = next(n.value.value for n in module.body if isinstance(n, ast.Assign)
              and any(isinstance(t, ast.Name) and t.id == 'verify' for t in n.targets))
r = subprocess.run(ssh + ['python3 -I -'], input=verify, text=True,
                   capture_output=True, check=True, timeout=60)
receipt = json.loads(r.stdout)
receipt['parallel_transport'] = plan
(D / 'media_cache_receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
print('ALL4_REMOTE_PINNED', flush=True)
# Launch the original frozen controller only after its original media checks can pass.
cmd = ('set -eu; test ! -e /root/prov/sweep_controller.pid; '
       'nohup setsid bash /root/sweep_controller.sh </dev/null '
       '>/root/prov/sweep_controller.out 2>&1 & controller_pid=$!; '
       'echo $controller_pid >/root/prov/sweep_controller.pid; '
       'kill -0 $controller_pid; echo CONTROLLER_PID=$controller_pid')
r = subprocess.run(ssh + [cmd], capture_output=True, text=True, check=True, timeout=30)
bundle = D / 'bundle.tar.gz'
with tarfile.open(bundle, 'r:gz') as tar:
    hashes = {n.name: hashlib.sha256(tar.extractfile(n).read()).hexdigest()
              for n in tar.getmembers() if n.isfile()}
upload = {
    'own_id': int((D/'own-id').read_text()), 'bundle_sha256': hashlib.sha256(bundle.read_bytes()).hexdigest(),
    'bytes': bundle.stat().st_size, 'files': hashes, 'stdout': r.stdout,
    'probe_sha256': json.loads((D / 'probe_hashes.json').read_text()),
    'product': '6e3fd3fd6d809ae2c2a3504cd8be1ecd29b2b31b',
    'guard': '9ff51a6e9432ed4d5f51636c7606360728de5b6c', 'tier3': 'a6456796e4af84d3b0c8bcb3558b63c2e85df42e',
    'native_fixture': '618f0431815d037d4226b642ae37e76967353e90',
    'measured': '48b027267a0e05c9ffdb4a049d34f4d719fd1837',
    'media_all_verified_before_controller': True, 'transport_receipt': receipt,
}
(D / 'upload_receipt.json').write_text(json.dumps(upload, indent=2) + '\n')
print(r.stdout, flush=True)
