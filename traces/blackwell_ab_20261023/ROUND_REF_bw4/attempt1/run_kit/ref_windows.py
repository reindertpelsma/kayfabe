"""Operator side, BW4 (D) REFERENCE lease: native windows-on-vast Windows on our own lease.

The template's installer prints the new Windows SSH host key on the console before it erases
the Linux disk; that key is pinned (one distinct key or refuse) and SSH to Windows is strict.
Usage (workspace = cwd's parent of this file's copy):
  ref_windows.py wait            pin the key, wait for SSH and setup complete (bounded)
  ref_windows.py run SCRIPT OUT TIMEOUT   copy SCRIPT to C:\\kf and run it, output to OUT
  ref_windows.py pull DEST       scp C:/kf/cap to DEST (private), verify the guest sha256 list
"""

import datetime as dt
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import time

D = Path(__file__).resolve().parent
MARKER = "Save this WINDOWS SSH HOST KEY before reboot:"
KEY = re.compile(r"^(ssh-ed25519 [A-Za-z0-9+/]{68})(?: \S+)?$")
CONSOLE = re.compile(r"^\[\s*[0-9.]+\]\s+\S+\[\d+\]:\s+")  # "[   40.85] bash[2415]: "
WAIT_SECONDS = 80 * 60
LEDGER = Path(
    "/workspace/bw4-wt/traces/blackwell_ab_20261023/ROUND_REF_bw4/attempt1/owned_instances.json"
)


def windows_host_key(log_text):
    lines = [CONSOLE.sub("", line.strip()) for line in log_text.splitlines()]
    keys = set()
    for index, line in enumerate(lines):
        if line.endswith(MARKER) and index + 1 < len(lines):
            match = KEY.match(lines[index + 1])
            if not match:
                raise ValueError("line after the host-key marker is not an ed25519 key")
            keys.add(match[1])
    if len(keys) > 1:
        raise ValueError("more than one Windows host key printed; refuse")
    return keys.pop() if keys else None


def known_hosts_line(alias, key):
    return f"{alias} {key}\n"


def setup_complete(text):
    try:
        state = json.loads(text.strip().lstrip("\ufeff"))
    except ValueError:
        return False
    return isinstance(state, dict) and state.get("complete") is True


def lease():
    iid = int((D / "own-id").read_text())
    conf = (D / "ssh.conf").read_text()
    host = re.search(r"HostName (\S+)", conf)[1]
    port = int(re.search(r"Port (\d+)", conf)[1])
    return iid, host, port


def win_conf(iid, host, port):
    alias = f"bw-ref-win-{iid}"
    text = (
        f"Host {alias}\n HostName {host}\n Port {port}\n User vast\n"
        " IdentityFile /root/.ssh/id_ed25519\n IdentitiesOnly yes\n ForwardAgent no\n"
        " BatchMode yes\n ConnectTimeout 20\n ServerAliveInterval 15\n ServerAliveCountMax 4\n"
        f" HostKeyAlias {alias}\n UserKnownHostsFile {D}/known_hosts_windows\n"
        " StrictHostKeyChecking yes\n"
    )
    (D / "ssh_windows.conf").write_text(text)
    return alias


def ssh(alias, command, timeout):
    return subprocess.run(
        ["ssh", "-F", str(D / "ssh_windows.conf"), alias, command],
        capture_output=True,
        text=True,
        timeout=timeout,
    )


def log_event(text):
    stamp = dt.datetime.now(dt.timezone.utc).isoformat()
    with (D / "ref_windows.log").open("a") as out:
        out.write(f"{stamp} {text}\n")
    print(stamp, text, flush=True)


def wait():
    iid, host, port = lease()
    alias = win_conf(iid, host, port)
    rows = json.loads(LEDGER.read_text())
    created = next(dt.datetime.fromisoformat(r["created_utc"]) for r in rows if int(r["id"]) == iid)
    deadline = created + dt.timedelta(seconds=WAIT_SECONDS)
    pinned = None
    while dt.datetime.now(dt.timezone.utc) < deadline:
        logs = subprocess.run(
            ["vastai", "logs", str(iid), "--tail", "20000"],
            capture_output=True,
            text=True,
            timeout=60,
        ).stdout
        key = windows_host_key(logs)
        if key and pinned and key != pinned:
            raise ValueError("Windows host key changed after pinning")
        if key and not pinned:
            pinned = key
            (D / "known_hosts_windows").write_text(known_hosts_line(alias, key))
            digest = hashlib.sha256(key.encode()).hexdigest()
            log_event("WINDOWS_HOSTKEY_PINNED sha256(line)=" + digest)
        if pinned:
            try:
                r = ssh(alias, "type C:\\ProgramData\\VastWindows\\state.json", 40)
                if r.returncode == 0 and setup_complete(r.stdout):
                    log_event("WINDOWS_SETUP_COMPLETE")
                    return 0
                log_event(f"WINDOWS_NOT_READY rc={r.returncode}")
            except subprocess.TimeoutExpired:
                log_event("WINDOWS_SSH_TIMEOUT")
        time.sleep(30)
    log_event("WINDOWS_WAIT_DEADLINE")
    return 75


def run(script, out, timeout):
    iid, host, port = lease()
    alias = win_conf(iid, host, port)
    ssh(alias, "powershell -NoProfile -Command New-Item -ItemType Directory -Force C:\\kf", 60)
    name = Path(script).name
    subprocess.run(
        ["scp", "-q", "-F", str(D / "ssh_windows.conf"), str(script), f"{alias}:C:/kf/{name}"],
        check=True,
        timeout=120,
    )
    command = f"powershell -NoProfile -ExecutionPolicy Bypass -File C:\\kf\\{name}"
    try:
        r = ssh(alias, command, timeout)
        text, rc = r.stdout + r.stderr, r.returncode
    except subprocess.TimeoutExpired as error:
        text, rc = (error.stdout or b"").decode(errors="replace") if error.stdout else "", 124
    Path(out).write_text(text)
    log_event(f"REF_RUN {name} rc={rc} bytes={len(text)}")
    return rc


def pull(dest):
    sys.path.insert(0, str(D))
    import capture_pack

    iid, host, port = lease()
    alias = win_conf(iid, host, port)
    dest = Path(dest)
    dest.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["scp", "-q", "-r", "-F", str(D / "ssh_windows.conf"), f"{alias}:C:/kf/cap", str(dest)],
        check=True,
        timeout=1800,
    )
    expected, complete = capture_pack.guest_manifest(dest)
    bad = []
    for name, (digest, size) in expected.items():
        path = dest / name
        if not path.is_file() or capture_pack.sha256(path) != digest or path.stat().st_size != size:
            bad.append(name)
    receipt = {"files": len(expected), "complete": complete, "bad": bad, "private": str(dest)}
    (dest.parent / "native_capture_receipt.json").write_text(json.dumps(receipt, indent=1) + "\n")
    log_event(f"REF_PULL files={len(expected)} complete={complete} bad={len(bad)}")
    return 0 if complete and not bad else 3


def main(argv):
    if argv[:1] == ["wait"]:
        return wait()
    if argv[:1] == ["run"] and len(argv) == 4:
        return run(argv[1], argv[2], int(argv[3]))
    if argv[:1] == ["pull"] and len(argv) == 2:
        return pull(argv[1])
    raise SystemExit(__doc__)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
