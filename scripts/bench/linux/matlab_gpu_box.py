#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Box-side steps of the first kf3 GPU test of the Linux proprietary image C (MATLAB). Run ON a Vast KVM box.

STATUS: LIVE as a one-off harness, 2026-11-17 (its first run is traces/matlab_gpu_20261117/; nothing here is qualified beyond that).

  matlab_gpu_box.py fetch  --url-file F [--work W]      download the image (presigned GET from F: {"url","size","sha256"}, mode 0600; the
                                                        URL is never printed), verify size and sha256, unpack, lay out the run dir W/img
  matlab_gpu_box.py driver --version V [--work W]       NVIDIA public CDN .run + sha256sum.txt of the same directory; verify; record Last-Modified
  matlab_gpu_box.py boot   --qemu Q --version V [--work W]   propimage_linux.py run --keep --gpu kf3 (offline clone, recorded clock)
  matlab_gpu_box.py install --version V [--work W]      propimage_linux.py install-driver (the .run is staged into the offline guest)
  matlab_gpu_box.py guest  [--timeout S] -- CMD...       one command in the guest over ssh; stdout and stderr to this stdout
  matlab_gpu_box.py guest-file --timeout S FILE          the file's text as a bash script on the guest's stdin
  matlab_gpu_box.py stop   [--work W]                    clean poweroff of the clone

Rules: no credential on the box except the one presigned URL file; the URL never goes to a log; text output only.
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
PROP = str(HERE / "propimage_linux.py")
CDN = ("https://us.download.nvidia.com/XFree86/Linux-x86_64/{v}", "https://us.download.nvidia.com/tesla/{v}")


def say(msg):
    print("[%s] %s" % (time.strftime("%FT%TZ", time.gmtime()), msg), flush=True)


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 22), b""):
            h.update(b)
    return h.hexdigest()


def download(url, dest, size=None, label="file", tries=6):
    """Resumable GET. Never prints the URL (it is a bearer); an error line carries the status or exception class only."""
    dest = Path(dest)
    part = Path(str(dest) + ".part")
    for attempt in range(tries):
        have = part.stat().st_size if part.exists() else 0
        req = urllib.request.Request(url, headers={"Range": "bytes=%d-" % have} if have else {})
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                if have and r.status != 206:
                    have = 0
                    part.unlink()
                lm = r.headers.get("Last-Modified")
                with open(part, "ab") as f:
                    t0 = time.monotonic()
                    n = 0
                    while True:
                        b = r.read(1 << 20)
                        if not b:
                            break
                        f.write(b)
                        n += len(b)
                        if time.monotonic() - t0 > 30:
                            say("%s: %d bytes" % (label, part.stat().st_size))
                            t0 = time.monotonic()
            if size is None or part.stat().st_size == size:
                os.replace(part, dest)
                return lm
            say("%s: short (%d of %s), retry" % (label, part.stat().st_size, size))
        except urllib.error.HTTPError as e:
            say("%s: HTTP %s on attempt %d" % (label, e.code, attempt + 1))
            if e.code in (403, 404):
                raise SystemExit("%s: HTTP %s (stop)" % (label, e.code))
        except Exception as e:  # noqa: BLE001 - class name only, a message can carry the URL
            say("%s: %s on attempt %d" % (label, type(e).__name__, attempt + 1))
        time.sleep(5)
    raise SystemExit("%s: download failed" % label)


def cmd_fetch(a):
    w = Path(a.work)
    spec = json.loads(Path(a.url_file).read_text())
    dl, img = w / "dl", w / "img"
    dl.mkdir(parents=True, exist_ok=True)
    z = dl / "image.zip"
    t0 = time.monotonic()
    download(spec["url"], z, spec["size"], "image.zip")
    say("DOWNLOAD_DONE bytes=%d seconds=%.0f" % (z.stat().st_size, time.monotonic() - t0))
    got = sha256_file(z)
    say("SHA256 %s want=%s %s" % (got, spec["sha256"], "OK" if got == spec["sha256"] else "MISMATCH"))
    if got != spec["sha256"]:
        return 1
    Path(a.url_file).unlink()  # the bearer is no longer needed
    t0 = time.monotonic()
    r = subprocess.run([sys.executable, PROP, "unpack", str(z), "--dir", str(w / "unp")], capture_output=True, text=True)
    say("UNPACK rc=%d %s seconds=%.0f" % (r.returncode, (r.stdout + r.stderr).strip()[-200:], time.monotonic() - t0))
    if r.returncode:
        return 1
    (img / "sealed").mkdir(parents=True, exist_ok=True)
    (img / "ssh").mkdir(parents=True, exist_ok=True)
    for n in ("image.qcow2", "packages.manifest.json", "packages.tsv", "seal_report.txt"):
        if (w / "unp" / n).exists():
            os.replace(w / "unp" / n, img / "sealed" / n)
    os.replace(w / "unp" / "state.json", img / "state.json")
    st = json.loads((img / "state.json").read_text())
    k = img / "ssh" / "id_ed25519"
    k.write_text(st["guest"]["ssh_private_key"])
    k.chmod(0o600)
    z.unlink()
    say("LAYOUT_DONE dir=%s ram_mb=%s smp=%s fb_mb=%s seal_utc=%s" % (img, st["qemu"]["ram_mb"], st["qemu"]["smp"], st["qemu"]["fb_mb"],
                                                                     st["clock"]["last_time_at_stop_utc"]))
    return 0


def cmd_driver(a):
    v = a.version
    require_ver = re.fullmatch(r"\d+\.\d+(\.\d+)?", v)
    if not require_ver:
        raise SystemExit("bad version")
    d = Path(a.work) / "drivers"
    d.mkdir(parents=True, exist_ok=True)
    name = "NVIDIA-Linux-x86_64-%s.run" % v
    for base in CDN:
        base = base.format(v=v)
        try:
            sums = urllib.request.urlopen(base + "/sha256sum.txt", timeout=60).read().decode()
        except Exception as e:  # noqa: BLE001
            say("sha256sum.txt at %s: %s" % (base, type(e).__name__))
            continue
        want = [l.split()[0] for l in sums.splitlines() if l.strip().endswith(name)]
        if len(want) != 1:
            say("checksum file at %s lists %d entries for %s" % (base, len(want), name))
            continue
        run = d / name
        lm = None
        if not (run.exists() and sha256_file(run) == want[0]):
            lm = download(base + "/" + name, run, None, name)
        else:
            lm = urllib.request.urlopen(urllib.request.Request(base + "/" + name, method="HEAD"), timeout=60).headers.get("Last-Modified")
        got = sha256_file(run)
        say("DRIVER %s sha256=%s want=%s %s last_modified=%s source=%s" % (name, got, want[0], "OK" if got == want[0] else "MISMATCH", lm, base))
        if got != want[0]:
            return 1
        import email.utils
        import datetime
        created = datetime.datetime(*email.utils.parsedate(lm)[:6]).strftime("%Y-%m-%dT%H:%M:%SZ") if lm else ""
        (d / (name + ".meta")).write_text(json.dumps({"sha256": got, "created_utc": created}))
        return 0
    raise SystemExit("DRIVER_NOT_FOUND")


def cmd_boot(a):
    r = subprocess.run([sys.executable, PROP, "run", "--dir", str(Path(a.work) / "img"), "--keep", "--gpu", "kf3", "--qemu", a.qemu,
                        "--driver-version", a.version], text=True)
    return r.returncode


def cmd_install(a):
    d = Path(a.work) / "drivers"
    run = d / ("NVIDIA-Linux-x86_64-%s.run" % a.version)
    meta = json.loads((d / (run.name + ".meta")).read_text())
    cmd = [sys.executable, PROP, "install-driver", "--dir", str(Path(a.work) / "img"), "--run", str(run), "--sha256", meta["sha256"]]
    if meta["created_utc"]:
        cmd += ["--created-utc", meta["created_utc"]]
    return subprocess.run(cmd, text=True).returncode


def _ssh_base(work):
    img = Path(work) / "img"
    st = json.loads((img / "state.json").read_text())
    return ["ssh", "-i", str(img / "ssh" / "id_ed25519"), "-p", str(st["ids"]["net"]["ssh_port"]), "-o", "IdentitiesOnly=yes",
            "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null", "-o", "BatchMode=yes", "-o", "LogLevel=ERROR",
            "-o", "ServerAliveInterval=30", "%s@127.0.0.1" % st["guest"]["user"]]


def cmd_guest(a):
    cmd = " ".join(a.cmd)
    try:
        return subprocess.run(_ssh_base(a.work) + [cmd], timeout=a.timeout).returncode
    except subprocess.TimeoutExpired:
        say("GUEST_CMD_TIMEOUT after %ds" % a.timeout)
        return 124


def cmd_guest_file(a):
    try:
        return subprocess.run(_ssh_base(a.work) + ["bash -s"], stdin=open(a.file), timeout=a.timeout).returncode
    except subprocess.TimeoutExpired:
        say("GUEST_CMD_TIMEOUT after %ds" % a.timeout)
        return 124


def cmd_stop(a):
    subprocess.run(_ssh_base(a.work) + ["sudo systemctl poweroff"], timeout=30)
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--work", default="/workspace/matlab_gpu")
    sub = ap.add_subparsers(dest="verb", required=True)
    s = sub.add_parser("fetch"); s.add_argument("--url-file", required=True)
    s = sub.add_parser("driver"); s.add_argument("--version", required=True)
    s = sub.add_parser("boot"); s.add_argument("--qemu", required=True); s.add_argument("--version", required=True)
    s = sub.add_parser("install"); s.add_argument("--version", required=True)
    s = sub.add_parser("guest"); s.add_argument("--timeout", type=int, default=600); s.add_argument("cmd", nargs="+")
    s = sub.add_parser("guest-file"); s.add_argument("--timeout", type=int, default=3600); s.add_argument("file")
    sub.add_parser("stop")
    a = ap.parse_args()
    return {"fetch": cmd_fetch, "driver": cmd_driver, "boot": cmd_boot, "install": cmd_install, "guest": cmd_guest,
            "guest-file": cmd_guest_file, "stop": cmd_stop}[a.verb](a)


if __name__ == "__main__":
    sys.exit(main())
