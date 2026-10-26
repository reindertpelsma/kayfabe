"""Wait (bounded, 6 min) for the owned box to accept SSH, then upload the bundle and start controller + lane.
Every probe prints the instance status and the ssh endpoint; no exception is swallowed. Any failure destroys the exact own id."""
import sys, json, time, datetime, subprocess, ipaddress
from pathlib import Path
sys.path.insert(0, '/workspace/bw-run')
from common_bw import *
iid = int(sys.argv[1]); src = sys.argv[2]
def now(): return datetime.datetime.now(datetime.timezone.utc)
r = next(x for x in json.loads(LEDGER.read_text()) if x['id'] == iid)
end = datetime.datetime.fromisoformat(r['created_utc']) + datetime.timedelta(minutes=6)
alias = 'bwlane-' + str(iid); conf = D / 'ssh.conf'

try:
    n = 0
    while now() < end:
        n += 1
        a = api(['show', 'instances']); x = next((x for x in a if x['id'] == iid), None)
        if x is None:
            raise RuntimeError('instance vanished from the API')
        st = {k: x.get(k) for k in ('actual_status', 'cur_state', 'intended_status', 'status_msg', 'public_ipaddr')}
        ports = x.get('ports') or {}
        mapped = (ports.get('22/tcp') or [{}])[0].get('HostPort')
        print('PROBE', n, now().isoformat(), json.dumps(st), 'ssh_port', mapped, flush=True)
        if x.get('public_ipaddr') and mapped:
            ip = str(ipaddress.ip_address(x['public_ipaddr'].strip()))
            conf.write_text(f"Host {alias}\n HostName {ip}\n Port {int(mapped)}\n User root\n IdentityFile /root/.ssh/id_ed25519\n IdentitiesOnly yes\n ForwardAgent no\n BatchMode yes\n ConnectTimeout 8\n ServerAliveInterval 15\n ServerAliveCountMax 3\n HostKeyAlias {alias}\n UserKnownHostsFile /dev/null\n StrictHostKeyChecking no\n")
            p = subprocess.run(['ssh', '-F', str(conf), alias, 'echo BW_SSH_READY'], capture_output=True, text=True, timeout=25)
            print('SSH_RC', p.returncode, (p.stderr or '').strip()[:200], flush=True)
            if p.returncode == 0 and 'BW_SSH_READY' in p.stdout:
                print('READINESS_PASS', iid, now().isoformat(), flush=True); break
        time.sleep(15)
    else:
        raise RuntimeError('no ssh answer within 6 minutes')
    subprocess.run(['python3', str(D / 'upload_bw.py'), str(iid), src], check=True)
except BaseException as e:
    print('DEPLOYMENT_STOP: destroying own id', iid, repr(e), flush=True)
    subprocess.run(['python3', str(D / 'destroy_owned.py'), str(iid)], check=True)
    raise
