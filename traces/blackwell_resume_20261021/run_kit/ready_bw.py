"""Wait (bounded) for the owned box to accept SSH, then upload the bundle and start controller + lane. Any failure destroys the exact own id."""
import sys, json, time, datetime, subprocess, ipaddress
from pathlib import Path
sys.path.insert(0, '/workspace/bw-run')
from common_bw import *
iid = int(sys.argv[1]); src = sys.argv[2]
r = next(x for x in json.loads(LEDGER.read_text()) if x['id'] == iid)
end = datetime.datetime.fromisoformat(r['created_utc']) + datetime.timedelta(minutes=10)
alias = 'bwlane-' + str(iid); conf = D / 'ssh.conf'
def write_conf():
    a = api(['show', 'instances']); x = next(x for x in a if x['id'] == iid)
    ip = str(ipaddress.ip_address(x['public_ipaddr'])); port = int(x['ports']['22/tcp'][0]['HostPort'])
    conf.write_text(f"Host {alias}\n HostName {ip}\n Port {port}\n User root\n IdentityFile /root/.ssh/id_ed25519\n IdentitiesOnly yes\n ForwardAgent no\n BatchMode yes\n ConnectTimeout 8\n ServerAliveInterval 15\n ServerAliveCountMax 3\n HostKeyAlias {alias}\n UserKnownHostsFile {D}/known_hosts\n StrictHostKeyChecking accept-new\n")
try:
    while datetime.datetime.now(datetime.timezone.utc) < end:
        try:
            write_conf()
            p = subprocess.run(['ssh', '-F', str(conf), alias, 'echo BW_SSH_READY'], capture_output=True, text=True, timeout=25)
            if p.returncode == 0 and 'BW_SSH_READY' in p.stdout:
                print('READINESS_PASS', iid, datetime.datetime.now(datetime.timezone.utc).isoformat(), flush=True); break
        except Exception as e:
            pass
        print('BOOT_READINESS_PENDING', iid, flush=True); time.sleep(20)
    else:
        raise RuntimeError('startup readiness deadline')
    subprocess.run(['python3', str(D / 'upload_bw.py'), str(iid), src], check=True)
except BaseException:
    print('DEPLOYMENT_STOP: destroying own id', iid, flush=True)
    subprocess.run(['python3', str(D / 'destroy_owned.py'), str(iid)], check=True)
    raise
