#!/usr/bin/env python3
"""Win11 base seal with a host-side TRIM (owner request 2026-11-15): record guest facts and the stop time, stop the guest cleanly, make the
free space of the NTFS volume read as zeros WITHOUT touching the working disk (a throwaway overlay is attached with qemu-nbd
--discard=unmap --detect-zeroes=unmap and wiped with ntfswipe), then write the flat compressed seal from that overlay. The working disk image.qcow2
stays exactly as the guest left it (the fallback). (ntfswipe on the unmounted volume, zero writes become qcow2 zero clusters.) Runs ON the pc as root. Nothing is uploaded.
  w_sealflow.py [--dry-run]
"""
import json, os, shutil, subprocess, sys, time
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import w_life as bl
from w_lib import *  # noqa

PI = bl.PI
dry = "--dry-run" in sys.argv


def run(cmd, **kw):
    print("+", " ".join(map(str, cmd)), flush=True)
    return subprocess.run(cmd, check=True, text=True, **kw)


def verify_zero(img):
    """Attach the sealed copy read-only, read the NTFS $Bitmap and count NON-ZERO bytes in every unused cluster."""
    dev = "/dev/nbd1"
    run(["qemu-nbd", "-r", "-c", dev, "-f", "qcow2", str(img)])
    try:
        time.sleep(3)
        subprocess.run(["partprobe", dev], capture_output=True)
        time.sleep(2)
        out = subprocess.run(["lsblk", "-bnlo", "NAME,SIZE,FSTYPE", dev], capture_output=True, text=True).stdout
        cands = sorted([l.split() for l in out.splitlines() if len(l.split()) == 3 and l.split()[2] == "ntfs"], key=lambda x: -int(x[1]))
        part = "/dev/" + cands[0][0]
        bm = subprocess.run(["ntfscat", "-f", part, "$Bitmap"], capture_output=True).stdout
        info = subprocess.run(["ntfsinfo", "-m", part], capture_output=True, text=True).stdout
        csz = int([l for l in info.splitlines() if "Cluster Size" in l][0].split(":")[1])
        ncl = int([l for l in info.splitlines() if "Volume Size in Clusters" in l][0].split(":")[1])
        fd = os.open(part, os.O_RDONLY)
        free = bad = badbytes = 0
        i = 0
        while i < ncl:
            if (bm[i >> 3] >> (i & 7)) & 1:
                i += 1
                continue
            j = i
            while j < ncl and not ((bm[j >> 3] >> (j & 7)) & 1) and j - i < 4096:
                j += 1
            buf = os.pread(fd, (j - i) * csz, i * csz)
            free += j - i
            if buf.count(0) != len(buf):
                bad += 1
                badbytes += len(buf) - buf.count(0)
            i = j
        os.close(fd)
        return {"cluster_size": csz, "free_clusters": free, "free_bytes": free * csz, "runs_with_nonzero": bad, "nonzero_bytes": badbytes}
    finally:
        subprocess.run(["qemu-nbd", "-d", dev], capture_output=True)


def main():
    st = bl.state()
    resume = "--resume-stopped" in sys.argv
    if not resume and not bl.qemu_pid():
        raise SystemExit("the install session guest is not running; this flow records facts over ssh first (or pass --resume-stopped)")
    free = free_gb()
    print("free GB", free)
    if free < 75:
        raise SystemExit("REFUSED: need >= 75 GB free for the flat copy (+20 floor is checked by the copy); free %.1f" % free)
    if dry:
        print("dry-run: would probe, stop, trim a throwaway overlay, convert"); return 0
    if resume:
        if bl.qemu_pid():
            raise SystemExit("--resume-stopped but the guest is running")
        rows = json.loads(bl.LIFE.read_text()) if bl.LIFE.exists() else []
        stops = [r for r in rows if r["step"] == "4a-stopped"]
        # without a lifecycle row the stop time is the one the last guest probe recorded into state.json just before the clean stop
        stop = stops[-1]["stop_utc"] if stops else st["clock"]["last_time_at_stop_utc"]
        st["clock"]["last_time_at_stop_utc"] = PI.fmt(PI.utc(stop))
        print("resuming from the stopped guest; recorded stop time", stop, flush=True)
    else:
        # 1. facts + stop time while the guest still runs (the same as w_life.py seal)
        shutil.copyfile(D / "state.json", D / "run/state.before_seal.json")
        guest = PI.run_guest_probe(st, str(bl.KEY))
        PI.fill_from_guest(st, guest)
        t_stop = guest["date_utc"]
        (D / "state.json").write_text(json.dumps(st, indent=2) + "\n")
        print("guest clock at stop:", t_stop, flush=True)
        bl.ssh_ps(st, bl.KEY, "Stop-Computer -Force", 60)
        for _ in range(90):
            if not bl.qemu_pid():
                break
            time.sleep(4)
        else:
            raise SystemExit("qemu still running after Stop-Computer; not killed")
        for _ in range(30):          # the install session wrapper (broker, swtpm) ends by itself
            if subprocess.run(["pgrep", "-f", "w_life.py session"], capture_output=True).returncode != 0:
                break
            time.sleep(2)
        st["clock"]["last_time_at_stop_utc"] = PI.fmt(PI.utc(t_stop))
        bl.row("4a-stopped", stop_utc=st["clock"]["last_time_at_stop_utc"], file=str(D / "image.qcow2"), bytes=(D / "image.qcow2").stat().st_size)
    # 2. throwaway overlay, TRIM on it
    sid = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    work = D / "runs" / ("trim_" + sid)
    work.mkdir(parents=True)
    trim = work / "trim.qcow2"
    run(["qemu-img", "create", "-q", "-f", "qcow2", "-b", str(D / "image.qcow2"), "-F", "qcow2", str(trim)])
    run(["modprobe", "nbd", "max_part=16"])
    dev = "/dev/nbd0"
    trimmed = False
    mnt = work / "mnt"
    mnt.mkdir()
    try:
        run(["qemu-nbd", "-c", dev, "--discard=unmap", "--detect-zeroes=unmap", "-f", "qcow2", str(trim)])
        time.sleep(3)
        run(["partprobe", dev])
        time.sleep(2)
        out = subprocess.run(["lsblk", "-bnlo", "NAME,SIZE,FSTYPE", dev], capture_output=True, text=True).stdout
        print(out)
        cands = [l.split() for l in out.splitlines() if len(l.split()) == 3 and l.split()[2] in ("ntfs", "BitLocker")]
        cands.sort(key=lambda x: -int(x[1]))
        if not cands:
            raise RuntimeError("no NTFS partition found")
        part = "/dev/" + cands[0][0]
        print("C: partition", part, cands[0][1])
        # ntfswipe zeroes the unused clusters, directory slack, MFT slack, file tails and undelete data of the UNMOUNTED volume; the
        # nbd export turns the zero writes into qcow2 zero clusters on the throwaway overlay (no allocation). The pagefile is wiped too (stale memory); not -l (journal).
        r = subprocess.run(["ntfswipe", "-u", "-d", "-m", "-t", "-s", "-p", part], capture_output=True, text=True)
        print(r.stdout[-1500:], r.stderr[-800:])
        if r.returncode != 0:
            raise RuntimeError("ntfswipe failed (volume not clean?): " + r.stderr.strip()[:300])
        trimmed = True
    except Exception as e:
        print("WIPE FAILED, NOT SEALING:", e, flush=True)
        subprocess.run(["umount", str(mnt)], capture_output=True)
        subprocess.run(["qemu-nbd", "-d", dev], capture_output=True)
        raise SystemExit(2)
    finally:
        subprocess.run(["umount", str(mnt)], capture_output=True)
        subprocess.run(["qemu-nbd", "-d", dev], capture_output=True)
    print("trim overlay allocated:", subprocess.run(["du", "-sh", str(trim)], capture_output=True, text=True).stdout.strip(), flush=True)
    # 3. flat compressed seal from the trimmed overlay
    dest = bl.SEALED / sid
    dest.mkdir(parents=True)
    t0 = time.time()
    run(bl.NICE + ["qemu-img", "convert", "-c", "-m", "4", "-O", "qcow2", str(trim), str(dest / "image.qcow2")])
    print("convert seconds", int(time.time() - t0), flush=True)
    PI.capture_files(st, str(D))
    (D / "state.json").write_text(json.dumps(st, indent=2) + "\n")
    shutil.copyfile(D / "state.json", dest / "state.json")
    sha_img = PI.sha256_file(dest / "image.qcow2")
    sha_st = PI.sha256_file(dest / "state.json")
    (dest / "manifest.json").write_text(json.dumps({"manifest_version": 1, "members": {"image.qcow2": sha_img, "state.json": sha_st}}, sort_keys=True))
    for f in dest.iterdir():
        f.chmod(0o444)
    bl.row("4-sealed", id=sid, stop_utc=st["clock"]["last_time_at_stop_utc"], file=str(dest / "image.qcow2"), bytes=(dest / "image.qcow2").stat().st_size,
           sha256=sha_img, state_sha256=sha_st, trimmed=trimmed, machine_guid=st["guest"].get("machine_guid"), ids=bl.ids_short(st),
           tpm_ovmf_sha256={k: v["sha256"][:16] for k, v in st["files"].items()})
    shutil.rmtree(work, ignore_errors=True)
    zc = verify_zero(dest / "image.qcow2")
    bl.row("4b-zero-check", id=sid, **zc)
    print("ZERO CHECK", json.dumps(zc), flush=True)
    print("SEALED %s trimmed=%s bytes=%d stop=%s" % (sid, trimmed, (dest / "image.qcow2").stat().st_size, st["clock"]["last_time_at_stop_utc"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
