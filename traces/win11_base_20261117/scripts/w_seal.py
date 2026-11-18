#!/usr/bin/env python3
"""Win11 base: seal the stopped disk read-only as image.base.qcow2, make the offline copy, verify it.
Stages (argv): fw | seal | pack | all.   Runs ON the pc as root, VM stopped.
 fw    enroll the boot-GOP signing certificate (public db.crt, read from the kfwin secureboot dir) into B's OVMF vars
 seal  qemu-img convert -c image.qcow2 -> image.base.qcow2 (0444), qemu-img compare, new image.qcow2 = overlay on the base
 pack  propimage.py pack (3-member STORED zip, embeds swtpm+OVMF files) -> zstd -> sha256s; stream check; zip deleted
"""
import hashlib
import json
import os
import sys
import time
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from w_lib import *  # noqa: E402,F401

PI = load_pi()
SB = Path("/workspace/pc1/winvm/secureboot")
BASE = D / "image.base.qcow2"
NICE = ["nice", "-n", "15", "ionice", "-c2", "-n7"]


def stopped():
    if qemu_pid():
        raise SystemExit("win11-base qemu is running; stop it first")


def fw():
    stopped()
    guid = (SB / "db.guid").read_text().strip()
    cert = SB / "db.crt"
    r = sh(["sbverify", "--cert", str(cert), str(D / "firmware/kf-gop.signed.efi")], check=False)
    log("fw: sbverify kf-gop.signed.efi against db.crt: rc=%d %s" % (r.returncode, (r.stdout + r.stderr).strip()))
    if r.returncode:
        raise SystemExit("the boot GOP efi is not signed by this db.crt; refusing to enroll")
    vars_ = D / "firmware/OVMF_VARS.fd"
    tmp = D / "firmware/OVMF_VARS.new"
    sh(["virt-fw-vars", "-i", str(D / "firmware/OVMF_VARS.before-gop.fd"), "-o", str(tmp), "--add-db", guid, str(cert)])
    vars_.write_bytes(tmp.read_bytes())
    tmp.unlink()
    pr = sh(["virt-fw-vars", "-i", str(vars_), "--print"], check=False).stdout
    (D / "run/ovmf_vars_print.txt").write_text(pr)
    log("fw: OVMF vars written (%d bytes); db entries mention guid: %s" % (vars_.stat().st_size, guid in pr))
    sh(["chown", "%s:%s" % (SU, SU), str(vars_)])


def seal():
    stopped()
    img = D / "image.qcow2"
    info = json.loads(sh(["qemu-img", "info", "--output=json", str(img)]).stdout)
    assert "backing-filename" not in info, "image.qcow2 already an overlay"
    used = info["actual-size"] / 1e9
    need_free(used, "compressed convert")
    tmp = D / "image.base.tmp"
    tmp.unlink(missing_ok=True)
    t0 = time.time()
    sh(NICE + ["qemu-img", "convert", "-c", "-m", "4", "-O", "qcow2", str(img), str(tmp)], timeout=7200)
    log("seal: convert -c done in %d s: %.1f GB -> %.1f GB" % (time.time() - t0, used, tmp.stat().st_size / 1e9))
    r = sh(NICE + ["qemu-img", "compare", str(img), str(tmp)], check=False, timeout=7200)
    log("seal: qemu-img compare rc=%d: %s" % (r.returncode, (r.stdout + r.stderr).strip()))
    if r.returncode:
        raise SystemExit("compare failed; raw image kept")
    chk = sh(["qemu-img", "check", str(tmp)], check=False)
    log("seal: qemu-img check rc=%d %s" % (chk.returncode, chk.stdout.strip().splitlines()[-1] if chk.stdout.strip() else chk.stderr.strip()[-100:]))
    tmp.rename(BASE)
    BASE.chmod(0o444)
    img.unlink()
    sh(["qemu-img", "create", "-q", "-f", "qcow2", "-b", str(BASE), "-F", "qcow2", str(img)])
    sh(["chown", "%s:%s" % (SU, SU), str(img)])
    log("seal: image.base.qcow2 sealed 0444 (%d bytes); image.qcow2 is now a thin overlay on it (the working disk for the owner's session)" % BASE.stat().st_size)


def pack():
    stopped()
    PACK_DIR.mkdir(parents=True, exist_ok=True)
    work = PACK_DIR / "work"
    work.mkdir(exist_ok=True)
    zpath = work / "image.zip"
    zpath.unlink(missing_ok=True)
    need_free(BASE.stat().st_size / 1e9 * 2.2, "zip + zst")
    t0 = time.time()
    r = sh(NICE + ["python3", "-B", "-I", str(D / "bin/propimage.py"), "pack", str(D / "state.json"), str(BASE), "--out", str(zpath), "--workdir", str(D)], timeout=7200)
    manifest = json.loads(r.stdout)
    log("pack: zip written in %d s: %.1f GB; members %s" % (time.time() - t0, zpath.stat().st_size / 1e9, sorted(manifest["members"])))
    zsha = PI.sha256_file(zpath)
    # stream-verify every member against the manifest without extracting
    with zipfile.ZipFile(zpath) as z:
        assert sorted(z.namelist()) == ["image.qcow2", "manifest.json", "state.json"]
        for name, want in manifest["members"].items():
            h = hashlib.sha256()
            with z.open(name) as f:
                for chunk in iter(lambda: f.read(1 << 22), b""):
                    h.update(chunk)
            assert h.hexdigest() == want, "member sha mismatch " + name
    log("pack: all members match manifest.json (streamed)")
    zst = PACK_DIR / "propimage_b_base.zip.zst"
    zst.unlink(missing_ok=True)
    t0 = time.time()
    sh(NICE + ["zstd", "-T4", "-3", "-q", str(zpath), "-o", str(zst)], timeout=7200)
    log("pack: zstd -3 done in %d s: zip %d bytes -> zst %d bytes" % (time.time() - t0, zpath.stat().st_size, zst.stat().st_size))
    sh(["zstd", "-t", "-q", str(zst)], timeout=3600)
    h = hashlib.sha256()
    p = subprocess.Popen(["zstd", "-dc", str(zst)], stdout=subprocess.PIPE)
    for chunk in iter(lambda: p.stdout.read(1 << 22), b""):
        h.update(chunk)
    p.wait()
    assert h.hexdigest() == zsha, "decompressed zst != zip"
    zsize = zpath.stat().st_size
    zst_sha = PI.sha256_file(zst)
    (PACK_DIR / "SHA256SUMS").write_text("%s  %s\n" % (zst_sha, zst.name))
    (PACK_DIR / "MANIFEST.txt").write_text(
        "propimage B offline copy (clean Windows 11 Pro base, GPU-less stage), %s\n"
        "file: %s  bytes=%d  sha256=%s\n"
        "inner zip (propimage.py pack format, 3 STORED members): bytes=%d sha256=%s\n"
        "members: %s\nimage.qcow2 bytes=%d\nrestore: zstd -d %s -o image.zip && python3 -B -I propimage.py unpack image.zip --dir DIR --workdir WORKDIR\n"
        "NOT uploaded anywhere. Contains the throwaway password, a throwaway key pair and the TPM state: private.\n"
        % (time.strftime("%Y-%m-%d %H:%M:%S%z"), zst.name, zst.stat().st_size, zst_sha, zsize, zsha,
           json.dumps(manifest["members"]), BASE.stat().st_size, zst.name))
    log("pack: stream check OK (zstd -dc sha256 == zip sha256); zst sha256 %s" % zst_sha)
    zpath.unlink()
    log("pack: plain zip deleted; kept %s" % zst)


if __name__ == "__main__":
    for st in (sys.argv[1:] or ["all"]):
        if st in ("fw", "all"):
            fw()
        if st in ("seal", "all"):
            seal()
        if st in ("pack", "all"):
            pack()
