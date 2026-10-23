from pathlib import Path
import datetime
import json
import subprocess

r = Path('/root/prov')
v = Path('/workspace/bw-capc-validation/kfwin/logs')
out = {'utc': datetime.datetime.now(datetime.timezone.utc).isoformat()}
p = r / 'sweep_controller.pid'
out['controller_alive'] = p.exists() and Path('/proc', p.read_text().strip(), 'exe').exists()
out['fatal'] = (r / 'XID_STOP').exists()
out['native_ready'] = (
    (r / 'sweep_native_grtier.log').exists()
    and 'GR_TIER_VERDICT=PASS' in (r / 'sweep_native_grtier.log').read_text()
)
out['native_ack'] = (r / 'N0_EXPORT_ACK').exists()
for name in [
    'box.log', 'driver.log', 'sweep_20261020_provision.log', 'sweep_host_die.log',
    'tree.log', 'measured_build.log', 'observed_b1_build.log', 'sweep_gates.log', 'sweep_bare595.log', 'sweep_cuda_host.log',
    'sweep_native_grtier.log', 'sweep_windows_install.log', 'bwr_bind.log',
    'arm_on_observe_b.log', 'arm_on_observe_b1.log', 'collect_on_observe_b.log',
    'collect_on_observe_b1.log',
]:
    p = r / name
    if p.exists():
        with p.open('rb') as f:
            f.seek(max(0, p.stat().st_size - 2048))
            lines = f.read().decode(errors='replace').splitlines()
        out[name] = lines[-1:] if name == 'measured_build.log' else lines[-2:]
p = v / 'install_run.log'
if p.exists():
    with p.open('rb') as f:
        f.seek(max(0, p.stat().st_size - 2048))
        out['install_life'] = f.read().decode(errors='replace').splitlines()[-2:]
for arm in ['on_observe_b', 'on_observe_b1']:
    p = v / (arm + '_run.log')
    if p.exists():
        lines = p.read_text().splitlines()
        out[arm + '_panic'] = [line for line in lines if 'GUEST_PANICKED' in line][-1:]
for name in ['native_receipt.json.accepted.json','XID_STOP']:
    p=r/name
    if p.exists():
        data=p.read_text()
        if len(data)<=32768:out[name]=json.loads(data)
print(json.dumps(out))
