"""Rent ONE GB202 KVM box for the Blackwell resume lane. Budget guard, ledger, auto-destroy armed first."""
import sys, json, datetime, subprocess, os
from pathlib import Path
sys.path.insert(0, '/workspace/bw-run')
from common_bw import *
minutes = int(sys.argv[2]) if len(sys.argv) > 2 else 180
src = sys.argv[1]; assert len(src) == 40
rows = json.loads(LEDGER.read_text()) if LEDGER.exists() else []
assert all(x.get('destroyed') for x in rows), 'a lease of this lane is still open'
live = api(['show', 'instances']); assert not live, 'account has a lease; act on own ids only'
credit = float(api(['show', 'user'])['credit'])
spent = sum(x.get('runtime_cost_estimate_usd', 0) for x in rows)
q = 'vms_enabled=true num_gpus=1 inet_down>200 disk_space>=140 cpu_cores_effective>=16 cpu_ram>=32'
offers = api(['search', 'offers', q, '-o', 'dph_total', '--limit', '80'])
want = int(sys.argv[3])
offers = [x for x in offers if int(x['id']) == want and x.get('machine_id') != 11051 and float(x.get('reliability', 0)) >= .98 and float(x.get('cpu_ram', 0)) >= 32000 and float(x.get('cpu_cores_effective', 0)) >= 16]
assert offers, 'no offer'
offer = offers[0]; rate = float(offer['dph_total'])
guard = max(rate * 1.15, rate + .10); reservation = guard * minutes / 60 + .5
assert spent + reservation < STOP and credit - reservation >= RESERVE, (spent, reservation, credit)
print('BUDGET', json.dumps({'credit': credit, 'spent': spent, 'reservation': reservation, 'minutes': minutes, 'stop': STOP, 'reserve': RESERVE}), flush=True)
print('OFFER', json.dumps({k: offer.get(k) for k in ['id', 'machine_id', 'gpu_name', 'dph_total', 'geolocation', 'reliability']}), flush=True)
p = subprocess.run(['vastai', 'create', 'instance', str(offer['id']), '--image', 'docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16', '--disk', '140', '--label', 'claude-bw-resume-20261021', '--raw'], capture_output=True, text=True, check=True)
r = json.loads(p.stdout); assert r.get('success'); iid = int(r['new_contract'])
x = {'id': iid, 'role': 'Blackwell resume GB202-die (RTX PRO 5000) Windows580.88: ON A2/A1 then OFF B1', 'offer': offer['id'], 'machine_id': offer.get('machine_id'), 'gpu_offer': offer['gpu_name'], 'quote_dph': rate, 'disk_gb_requested': 140, 'created_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'product_head': src, 'credit_before': credit, 'lane_start_credit': LANE_START_CREDIT, 'budget_stop': STOP, 'credit_reserve': RESERVE, 'reservation_usd': reservation, 'auto_destroy_minutes': minutes, 'destroyed': False}
rows.append(x); LEDGER.write_text(json.dumps(rows, indent=2) + '\n')
try:
    result = subprocess.check_output(['bash', '/root/night/arm_autodestroy.sh', str(iid), str(minutes)], text=True)
    pid = int(Path('/root/night/autodestroy-' + str(iid) + '.pid').read_text()); os.kill(pid, 0)
    x.update(auto_destroy_pid=pid, auto_destroy_verified=True, auto_destroy_armed_utc=datetime.datetime.now(datetime.timezone.utc).isoformat())
    LEDGER.write_text(json.dumps(rows, indent=2) + '\n'); print(result.strip(), flush=True)
except BaseException:
    subprocess.run(['vastai', 'destroy', 'instance', str(iid), '-y'], capture_output=True); raise
(D / 'own-id').write_text(str(iid) + '\n'); print('OWN_ID', iid, flush=True)
