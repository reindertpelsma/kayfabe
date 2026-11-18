#!/usr/bin/env python3
"""Win11 base lifecycle, one command per step (owner lifecycle 2026-11-15). Runs ON the pc as root; PROP_DIR=/workspace/pc1/winvm/win11_base.
Every step appends a row to PROP_DIR/lifecycle.json (UTC time, file, bytes, sha256, ids). Nothing is uploaded anywhere.
UNTESTED where it needs the GPU lock or the owner's desktop (marked); the GPU-free paths are exercised by --dry-run and by b_preflight.py.

  w_life.py session   [--mode setup|install] [--net on|off] [--cdrom ISO ...] [--dry-run]   step 2/3: the working disk image.qcow2, fixed identity, real clock
  w_life.py seal      [--stop-utc T]            step 4: record stop time (ssh), clean stop, flat copy -> sealed/<ID>/ (image.qcow2 ro, state.json, manifest.json)
  w_life.py checkpoint NAME                     step 1c: flat read-only copy of the stopped working disk (+ state with the live tpm/OVMF files embedded) -> checkpoints/NAME/; image.qcow2 becomes a new overlay on it
  w_life.py offline-run ID [--no-gpu] [--dry-run]  step 5: a COPY of the sealed image, restrict=on, RTC = recorded stop time + gap (propimage.py boot clock)
  w_life.py check     ID [--apps a,b] [--no-gpu]   step 6 (7 = its end; again as step 11 "CI-like test of the backup after the original is discarded"): propimage_check.py on the sealed dir (GPU), or the GPU-free preflight (--no-gpu); stops the guest
  w_life.py signout   ID [--dry-run]            step 8: the ORIGINAL disk (image.qcow2 + live tpm/firmware) ONLINE, real clock, window: the owner signs out / unregisters (OWNER STEP; needs a sealed ID first)
  w_life.py signout-done ID --confirm TEXT [--discard-original]   step 9/10: record the owner's confirmation (no secrets in TEXT); --discard-original deletes the original disk (step 10)
  w_life.py profile-list ID                     step 9a: GPU-free offline boot of a throwaway copy (ID = a sealed id, or "original" = the signed-out original disk); list NAMES (no contents) of account-related files/credential targets left in the profiles
  w_life.py discard   RUNDIR                    delete a throwaway copy made by offline-run
  w_life.py pack      ID                        step 12: propimage.py pack of the sealed dir -> zstd in PACK_DIR/<ID>/ (no upload)
  w_life.py table                               the recorded table
"""
import argparse
import fcntl
import hashlib
import json
import os
import shutil
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from w_lib import *  # noqa: E402,F401

PI = load_pi()
LIFE = D / "lifecycle.json"
SEALED = D / "sealed"
RUNS = D / "runs"
LOCK = "/root/prov/pc_gpu.lock"
KEY = D / "keys/id_ed25519"
BROKER = os.environ.get("PROP_BROKER", "/workspace/bench/pcdisp_20261112/bin/nvkvm-display-broker")
BSOCK = "/run/user/1000/nvkvm/display-b.sock"      # NOT the owner's display.sock
SRUN = "/run/user/1000"
NICE = ["nice", "-n", "10", "ionice", "-c2", "-n7"]


def utcnow():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def activelog(msg):
    with open("/root/prov/PC_ACTIVE.log", "a") as f:
        f.write("%s win11base %s\n" % (utcnow(), msg))


def state():
    st = json.loads((D / "state.json").read_text())
    PI.validate_state(st)
    assert PI.seed_matches(st)
    return st


def ids_short(st):
    i = st["ids"]
    return "uuid=%s serial=%s mac=%s" % (i["smbios"]["uuid"], i["smbios"]["serial"], i["nics"][0]["mac"])


def row(step, **kw):
    rows = json.loads(LIFE.read_text()) if LIFE.exists() else []
    rows.append(dict(step=step, utc=utcnow(), **kw))
    LIFE.write_text(json.dumps(rows, indent=1) + "\n")
    log("lifecycle row: %s %s" % (step, json.dumps(kw)[:300]), "life.log")


def gtk_env():
    return ["env", "XDG_RUNTIME_DIR=" + SRUN, "WAYLAND_DISPLAY=wayland-0", "GDK_BACKEND=wayland", "DBUS_SESSION_BUS_ADDRESS=unix:path=%s/bus" % SRUN]


def kf3_env():
    env = {}
    for line in (D / "bin/kf3_flags.sample.env").read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            assert __import__("re").fullmatch(r"KF3_[A-Z0-9_]+=\d+", line), line
            k, v = line.split("=")
            env[k] = v
    env["KF3_RPC_TRACE"] = "1"
    return env


def banner(net_on):
    if net_on:
        print("!!! THIS GUEST IS ONLINE (restrict=off): owner's app-install session. The sealed image is only ever booted OFFLINE (restrict=on).")


def run_vm(argv, workdir, mode, env_extra=None, net_on=False, kf3=False, as_ubuntu_gtk=False):
    """Start swtpm + (broker) + qemu for a foreground session; wait until QEMU exits."""
    wd = Path(workdir)
    (wd / "run").mkdir(exist_ok=True)
    start_swtpm(wd)
    procs = []
    lockfd = None
    try:
        if kf3:
            lockfd = os.open(LOCK, os.O_RDWR | os.O_CREAT)
            try:
                fcntl.flock(lockfd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise SystemExit("GPU LOCK HELD by another session: not waiting")
            sh(["install", "-d", "-o", SU, "-m", "0700", SRUN + "/nvkvm"])
            Path(BSOCK).unlink(missing_ok=True)
            procs.append(subprocess.Popen(["runuser", "-u", SU, "--"] + gtk_env()[:1] + gtk_env()[1:] +
                                          [BROKER, "--socket", BSOCK, "--backend", "auto", "--persist", "--title", "kayfabe B (propimage)"],
                                          stdout=open(wd / "run/broker.log", "w"), stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL))
            for _ in range(50):
                if Path(BSOCK).exists():
                    break
                time.sleep(0.2)
            assert Path(BSOCK).exists(), "broker made no socket"
        activelog("START %s session (%s) wd=%s" % (mode, "online" if net_on else "offline", wd))
        env = dict(os.environ, **(kf3_env() if kf3 else {}))
        if kf3:
            cmd = NICE + argv
        else:
            cmd = ["runuser", "-u", SU, "--"] + (gtk_env() if as_ubuntu_gtk else ["env"]) + NICE + argv
        p = subprocess.Popen(cmd, env=env, stdout=open(wd / "run/qemu.log", "ab"), stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
        procs.append(p)
        with open(wd / "run/marker.txt", "a") as f:
            f.write("START %s %s pid %d\n" % (mode, utcnow(), p.pid))
        p.wait()
        with open(wd / "run/marker.txt", "a") as f:
            f.write("END %s %s rc %d\n" % (mode, utcnow(), p.returncode))
        return p.returncode
    finally:
        for q in reversed(procs):
            if q.poll() is None:
                q.terminate()
        activelog("END %s session" % mode)
        if lockfd is not None:
            fcntl.flock(lockfd, fcntl.LOCK_UN)


def cmd_session(a, label=("2-session-start", "3-session-end")):
    st = state()
    if qemu_pid():
        raise SystemExit("an win11-base qemu is already running")
    need_free(10, "session growth")
    disk = D / "image.qcow2"
    cd = [str(Path(c).resolve()) for c in a.cdrom]
    if a.mode == "setup":
        argv = PI.build_argv(st, "setup", str(disk), str(D), qemu=DISTRO_QEMU, cdroms=cd)
        gtk = True
    else:
        argv = PI.build_argv(st, "install", str(disk), str(D), qemu=KF_QEMU, broker_sock=BSOCK, gop_efi=str(D / "firmware/kf-gop.signed.efi"),
                             uid=1000, display="broker", cdroms=cd)
        gtk = False
    if a.net == "off":
        argv = [x.replace("restrict=off", "restrict=on") for x in argv]
    argv = ["propimage-w11-" + a.mode if x.startswith("propimage-") and x in ("propimage-setup", "propimage-install") else x for x in argv]
    print("identity:", ids_short(st))
    print("CMD:", shlex.join(argv))
    if a.dry_run:
        return 0
    banner(a.net == "on")
    row(label[0], mode=a.mode, net=a.net, disk="image.qcow2 (overlay on image.base.qcow2)")
    rc = run_vm(argv, D, a.mode, net_on=a.net == "on", kf3=a.mode == "install", as_ubuntu_gtk=gtk)
    row(label[1], rc=rc, disk_bytes=(D / "image.qcow2").stat().st_size)
    return 0


def cmd_seal(a):
    st = state()
    t_stop = a.stop_utc
    if qemu_pid():
        # guide step 8: record the facts and the stop time while the guest still runs, then stop it cleanly
        shutil.copyfile(D / "state.json", D / "run/state.before_seal.json")
        guest = PI.run_guest_probe(st, str(KEY))
        PI.fill_from_guest(st, guest)
        t_stop = t_stop or guest["date_utc"]
        if a.stop_utc:
            st["clock"]["last_time_at_stop_utc"] = PI.fmt(PI.utc(a.stop_utc))
        (D / "state.json").write_text(json.dumps(st, indent=2) + "\n")
        ssh_ps(st, KEY, "Stop-Computer -Force", 60)
        for _ in range(90):
            if not qemu_pid():
                break
            time.sleep(4)
        else:
            raise SystemExit("qemu still running after Stop-Computer; not killed")
    else:
        # guest already stopped: no ssh. The stop time must be given, or defaults to the last write of the disk (UTC)
        if not t_stop:
            t_stop = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime((D / "image.qcow2").stat().st_mtime))
            print("NOTE: guest was already stopped; stop time = last write of image.qcow2 (%s). Pass --stop-utc to override." % t_stop)
        st["clock"]["last_time_at_stop_utc"] = PI.fmt(PI.utc(t_stop))
        if not st["guest"].get("machine_guid"):
            print("NOTE: machine_guid/ek_pub_hash were recorded at the base build and are unchanged by app installs.")
    sid = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    dest = SEALED / sid
    need_free((D / "image.qcow2").stat().st_size / 1e9 + 15, "flat compressed copy")
    dest.mkdir(parents=True)
    sh(NICE + ["qemu-img", "convert", "-c", "-m", "4", "-O", "qcow2", str(D / "image.qcow2"), str(dest / "image.qcow2")], timeout=14400)
    PI.capture_files(st, str(D))
    (D / "state.json").write_text(json.dumps(st, indent=2) + "\n")
    shutil.copyfile(D / "state.json", dest / "state.json")
    sha_img = PI.sha256_file(dest / "image.qcow2")
    sha_st = PI.sha256_file(dest / "state.json")
    (dest / "manifest.json").write_text(json.dumps({"manifest_version": 1, "members": {"image.qcow2": sha_img, "state.json": sha_st}}, sort_keys=True))
    for f in dest.iterdir():
        f.chmod(0o444)
    row("4-sealed", id=sid, stop_utc=st["clock"]["last_time_at_stop_utc"], file=str(dest / "image.qcow2"), bytes=(dest / "image.qcow2").stat().st_size,
        sha256=sha_img, state_sha256=sha_st, machine_guid=st["guest"].get("machine_guid"), ids=ids_short(st),
        tpm_ovmf_sha256={k: v["sha256"][:16] for k, v in st["files"].items()})
    print("SEALED %s: stop time %s; image sha256 %s" % (sid, st["clock"]["last_time_at_stop_utc"], sha_img))
    return 0


def cmd_checkpoint(a):
    st = state()
    if qemu_pid():
        raise SystemExit("an win11-base qemu is running; stop it first")
    img = D / "image.qcow2"
    dest = D / "checkpoints" / a.name
    need_free(img.stat().st_size / 1e9 + 15, "flat compressed copy")
    dest.mkdir(parents=True)
    sh(NICE + ["qemu-img", "convert", "-c", "-m", "4", "-O", "qcow2", str(img), str(dest / "image.qcow2")], timeout=14400)
    r = sh(NICE + ["qemu-img", "compare", str(img), str(dest / "image.qcow2")], check=False, timeout=14400)
    if r.returncode:
        raise SystemExit("compare failed: " + r.stdout + r.stderr)
    PI.capture_files(st, str(D))
    (D / "state.json").write_text(json.dumps(st, indent=2) + "\n")
    shutil.copyfile(D / "state.json", dest / "state.json")
    sha_img, sha_st = PI.sha256_file(dest / "image.qcow2"), PI.sha256_file(dest / "state.json")
    (dest / "manifest.json").write_text(json.dumps({"manifest_version": 1, "members": {"image.qcow2": sha_img, "state.json": sha_st}}, sort_keys=True))
    for f in dest.iterdir():
        f.chmod(0o444)
    img.unlink()
    sh(["qemu-img", "create", "-q", "-f", "qcow2", "-b", str(dest / "image.qcow2"), "-F", "qcow2", str(img)])
    sh(["chown", "%s:%s" % (SU, SU), str(img)])
    row("1c-checkpoint-" + a.name, file=str(dest / "image.qcow2"), bytes=(dest / "image.qcow2").stat().st_size, sha256=sha_img, state_sha256=sha_st,
        stop_utc=st["clock"]["last_time_at_stop_utc"], machine_guid=st["guest"].get("machine_guid"), ids=ids_short(st),
        note="working disk image.qcow2 is now a new overlay on this checkpoint (image.base.qcow2 stays the GPU-less base)")
    print("checkpoint", dest, sha_img)
    return 0


def load_sealed(sid):
    d = SEALED / sid
    st = json.loads((d / "state.json").read_text())
    PI.validate_state(st, require_password=True)
    assert PI.seed_matches(st)
    m = json.loads((d / "manifest.json").read_text())["members"]
    for n, want in m.items():
        assert PI.sha256_file(d / n) == want, "sealed copy member sha mismatch: " + n
    return d, st


def prepare_copy(sid, tag):
    """A COPY: fresh run dir with restored tpm/firmware from the sealed state and a throwaway overlay of the sealed image."""
    d, st = load_sealed(sid)
    rd = RUNS / ("%s_%s_%s" % (sid, tag, time.strftime("%H%M%S")))
    rd.mkdir(parents=True)
    PI.restore_files(st, str(rd))
    shutil.copyfile(D / "firmware/kf-gop.signed.efi", rd / "firmware" / "kf-gop.signed.efi") if (D / "firmware/kf-gop.signed.efi").exists() else None
    (rd / "run").mkdir()
    clone = rd / "run/clone.qcow2"
    sh(["qemu-img", "create", "-q", "-f", "qcow2", "-b", str(d / "image.qcow2"), "-F", "qcow2", str(clone)])
    sh(["chown", "-R", "%s:%s" % (SU, SU), str(rd)])
    return d, st, rd, clone


def cmd_offline_run(a):
    d, st, rd, clone = prepare_copy(a.id, "offline")
    mode = "offline"
    if a.no_gpu:      # offline without the GPU: faked clock + restrict=on on the distro qemu (window)
        argv = PI.build_argv(st, "setup", str(clone), str(rd), qemu=DISTRO_QEMU)
        base = PI.boot_clock(st, "offline")
        argv = [x if not x.startswith("base=utc") else "base=%s,clock=vm,driftfix=slew" % base.strftime("%Y-%m-%dT%H:%M:%S") for x in argv]
        argv = [x.replace("restrict=off", "restrict=on") for x in argv]
    else:
        argv = PI.build_argv(st, "offline", str(clone), str(rd), qemu=KF_QEMU, broker_sock=BSOCK, gop_efi=str(rd / "firmware/kf-gop.signed.efi"), uid=1000)
    argv = ["propimage-w11-" + mode if x in ("propimage-setup", "propimage-offline") else x for x in argv]
    s = " ".join(argv)
    assert "restrict=on" in s and str(clone) in s
    print("copy dir:", rd)
    print("identity:", ids_short(st))
    print("CMD:", shlex.join(argv))
    if a.dry_run:
        return 0
    row("5-offline-start", id=a.id, run=str(rd), net="off(restrict=on)", rtc=PI.fmt(PI.boot_clock(st, "offline")))
    rc = run_vm(argv, rd, mode, net_on=False, kf3=not a.no_gpu, as_ubuntu_gtk=a.no_gpu)
    row("7-offline-end", run=str(rd), rc=rc)
    print("throwaway copy kept at %s; delete with: w_life.py discard %s" % (rd, rd))
    return 0


def cmd_check(a):
    d, st = load_sealed(a.id)
    if a.no_gpu:
        print("check --no-gpu is not a separate test: use `offline-run ID --no-gpu` (boots a throwaway copy offline with the faked clock) and b_preflight.py-style checks; the app launches need the GPU.")
        return 0
    lockfd = os.open(LOCK, os.O_RDWR | os.O_CREAT)
    try:
        fcntl.flock(lockfd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        raise SystemExit("GPU LOCK HELD by another session: not waiting")
    work = RUNS / ("check_%s_%s" % (a.id, time.strftime("%H%M%S")))
    out = D / "evidence" / ("check_%s_%s" % (a.id, time.strftime("%H%M%S")))
    activelog("START propimage_check on sealed %s" % a.id)
    try:
        cmd = ["python3", "-B", "-I", str(D / "bin/propimage_check.py"), "--dir", str(d), "--work", str(work), "--out", str(out),
               "--qemu", KF_QEMU, "--flags", str(D / "bin/kf3_flags.sample.env"), "--broker", BROKER, "--gop-efi", str(D / "firmware/kf-gop.signed.efi")]
        if a.apps:
            cmd += ["--apps", a.apps]
        r = subprocess.run(NICE + cmd)
        row("6-check", id=a.id, rc=r.returncode, out=str(out))
        return r.returncode
    finally:
        activelog("END propimage_check")
        fcntl.flock(lockfd, fcntl.LOCK_UN)


def cmd_signout(a):
    load_sealed(a.id)           # the backup must exist and verify BEFORE any logout
    a.mode, a.net, a.cdrom = "setup", "on", []
    print("OWNER STEP: sign out / unregister every app in this window (online). The sealed copy %s is NOT touched." % a.id)
    return cmd_session(a, ("8-signout-start", "8-signout-end"))


def cmd_signout_done(a):
    load_sealed(a.id)
    if qemu_pid():
        raise SystemExit("an win11-base qemu is running")
    row("9-signout-confirmed", id=a.id, owner_confirmation=a.confirm[:200], note="owner-observed; the tool cannot see vendor-side state")
    if a.discard_original:
        img = D / "image.qcow2"
        img.unlink(missing_ok=True)
        row("10-original-discarded", id=a.id, note="image.qcow2 (the original, signed-out disk) deleted; live tpm/ and firmware/ left in place")
        print("original disk deleted. image.qcow2 must be re-created before another session: qemu-img create -f qcow2 -b %s/<ID>/image.qcow2 -F qcow2 image.qcow2" % SEALED)
    return 0


LIST_PS = r"""
$pat='adobe|creative ?cloud|oobe|\bims\b|identity|token|credential|licen[sc]|login|signin|steam|3dmark|ul benchmarks|solidworks|blackmagic|resolve|cookie|session'
$o=[ordered]@{}
$roots=@('C:\Users','C:\ProgramData')
$o.files=@(foreach($r in $roots){ Get-ChildItem -LiteralPath $r -Recurse -Force -Depth 6 -ErrorAction SilentlyContinue | Where-Object { $_.FullName -notmatch '\\Windows\\|\\Microsoft\\(Windows|Edge|Office)\\|\\Packages\\' -and $_.Name -match $pat } | Select-Object -First 400 | ForEach-Object { "$($_.FullName) $(if($_.PSIsContainer){'<dir>'}else{$_.Length})" } })
$o.cmdkey_targets=@(cmdkey /list | Select-String 'Target:' | ForEach-Object { $_.ToString().Trim() })
$o | ConvertTo-Json -Depth 3
"""


def prepare_original_copy():
    """The signed-out ORIGINAL disk as a throwaway overlay, with a copy of the live tpm/firmware (the original files stay untouched)."""
    st = state()
    rd = RUNS / ("original_proflist_%s" % time.strftime("%H%M%S"))
    rd.mkdir(parents=True)
    shutil.copytree(D / "tpm", rd / "tpm")
    shutil.copytree(D / "firmware", rd / "firmware")
    (rd / "run").mkdir()
    clone = rd / "run/clone.qcow2"
    sh(["qemu-img", "create", "-q", "-f", "qcow2", "-b", str(D / "image.qcow2"), "-F", "qcow2", str(clone)])
    sh(["chown", "-R", "%s:%s" % (SU, SU), str(rd)])
    return D, st, rd, clone


def cmd_profile_list(a):
    d, st, rd, clone = prepare_original_copy() if a.id == "original" else prepare_copy(a.id, "proflist")
    if a.id == "original":
        st["clock"]["last_time_at_stop_utc"] = st["clock"]["last_time_at_stop_utc"] or utcnow()
    argv = PI.build_argv(st, "setup", str(clone), str(rd), qemu=DISTRO_QEMU)
    base = PI.boot_clock(st, "offline")
    argv = [x if not x.startswith("base=utc") else "base=%s,clock=vm,driftfix=slew" % base.strftime("%Y-%m-%dT%H:%M:%S") for x in argv]
    argv = [x.replace("restrict=off", "restrict=on") for x in argv]
    argv = ["propimage-w11-proflist" if x == "propimage-setup" else x for x in argv]
    k = argv.index("gtk")
    argv[k] = "none"
    assert "restrict=on" in " ".join(argv)
    key = rd / "id_kf"
    key.write_text(st["guest"]["ssh_private_key"])
    key.chmod(0o600)
    start_swtpm(rd)
    start_qemu(argv, rd)
    out = D / "evidence" / ("profile_list_%s.txt" % a.id)
    try:
        t0 = time.time()
        while time.time() - t0 < 600:
            try:
                p = sh(ssh_base(st, key) + ["echo", "up"], check=False, timeout=30)
                if p.returncode == 0:
                    break
            except subprocess.TimeoutExpired:
                pass
            time.sleep(8)
        else:
            raise SystemExit("ssh timeout")
        r = ssh_ps(st, key, LIST_PS, 600)
        out.write_text(r.stdout)
        row("9a-profile-list", id=a.id, file=str(out), bytes=len(r.stdout), note="names only, offline throwaway copy; private, stays on the pc")
        print("wrote", out)
    finally:
        ssh_ps(st, key, "Stop-Computer -Force", 60)
        for _ in range(60):
            if not qemu_pid():
                break
            time.sleep(4)
        key.unlink(missing_ok=True)
    return 0


def cmd_discard(a):
    p = Path(a.rundir).resolve()
    if RUNS not in p.parents:
        raise SystemExit("REFUSED: only directories directly under %s" % RUNS)
    if qemu_pid():
        raise SystemExit("an win11-base qemu is running; stop it first")
    shutil.rmtree(p)
    print("deleted", p)


def cmd_pack(a):
    d, st = load_sealed(a.id)
    out = PACK_DIR / a.id
    out.mkdir(parents=True, exist_ok=True)
    need_free((d / "image.qcow2").stat().st_size / 1e9 * 2.2, "zip + zst")
    zp = out / "image.zip"
    work = RUNS / ("pack_" + a.id)
    work.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(d / "state.json", work / "state.json")     # pack must not touch the sealed copy
    r = sh(NICE + ["python3", "-B", "-I", str(D / "bin/propimage.py"), "pack", str(work / "state.json"), str(d / "image.qcow2"), "--out", str(zp)], timeout=14400)
    mf = json.loads(r.stdout)
    zst = out / ("propimage_b_%s.zip.zst" % a.id)
    sh(NICE + ["zstd", "-T4", "-3", "-q", str(zp), "-o", str(zst)], timeout=14400)
    sh(["zstd", "-t", "-q", str(zst)], timeout=3600)
    zsha = PI.sha256_file(zp)
    h = hashlib.sha256()
    p = subprocess.Popen(["zstd", "-dc", str(zst)], stdout=subprocess.PIPE)
    for chunk in iter(lambda: p.stdout.read(1 << 22), b""):
        h.update(chunk)
    p.wait()
    assert h.hexdigest() == zsha
    sha = PI.sha256_file(zst)
    (out / "SHA256SUMS").write_text("%s  %s\n" % (sha, zst.name))
    zsize = zp.stat().st_size
    zp.unlink()
    shutil.rmtree(work)
    row("12-packed", id=a.id, file=str(zst), bytes=zst.stat().st_size, sha256=sha, inner_zip_bytes=zsize, inner_zip_sha256=zsha, members=mf["members"],
        upload="none (owner decides about R2)")
    print("PACKED", zst, sha)
    return 0


def cmd_table(a):
    rows = json.loads(LIFE.read_text()) if LIFE.exists() else []
    print("| step | utc | detail |\n|---|---|---|")
    for r in rows:
        d = {k: v for k, v in r.items() if k not in ("step", "utc")}
        print("| %s | %s | %s |" % (r["step"], r["utc"], json.dumps(d)))
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = ap.add_subparsers(dest="verb", required=True)
    p = sp.add_parser("session"); p.add_argument("--mode", choices=("setup", "install"), default="setup"); p.add_argument("--net", choices=("on", "off"), default="on")
    p.add_argument("--cdrom", action="append", default=[]); p.add_argument("--dry-run", action="store_true")
    p = sp.add_parser("checkpoint"); p.add_argument("name")
    p = sp.add_parser("seal"); p.add_argument("--stop-utc")
    p = sp.add_parser("offline-run"); p.add_argument("id"); p.add_argument("--no-gpu", action="store_true"); p.add_argument("--dry-run", action="store_true")
    p = sp.add_parser("check"); p.add_argument("id"); p.add_argument("--apps"); p.add_argument("--no-gpu", action="store_true")
    p = sp.add_parser("signout"); p.add_argument("id"); p.add_argument("--dry-run", action="store_true")
    p = sp.add_parser("signout-done"); p.add_argument("id"); p.add_argument("--confirm", required=True); p.add_argument("--discard-original", action="store_true")
    p = sp.add_parser("profile-list"); p.add_argument("id")
    p = sp.add_parser("discard"); p.add_argument("rundir")
    p = sp.add_parser("pack"); p.add_argument("id")
    sp.add_parser("table")
    a = ap.parse_args()
    return {"session": cmd_session, "checkpoint": cmd_checkpoint, "seal": cmd_seal, "offline-run": cmd_offline_run, "check": cmd_check,
            "signout": cmd_signout, "signout-done": cmd_signout_done, "profile-list": cmd_profile_list, "discard": cmd_discard, "pack": cmd_pack, "table": cmd_table}[a.verb](a)


if __name__ == "__main__":
    sys.exit(main())
