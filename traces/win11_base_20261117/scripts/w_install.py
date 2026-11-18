#!/usr/bin/env python3
"""Win11 base, step 2: unattended Windows install in setup mode (distro QEMU, headless, restrict=on), up to SSH.
Deviations from propimage_run.sh setup (all logged): headless (-display none; screens by QMP screendump) so the owner's
desktop is untouched; restrict=on (the build guest is never online); QEMU at nice 19; name propimage-w11-setup.
Presses a key only in the first 45 s (the 'Press any key to boot from CD' prompt); never on later reboots (it would restart Setup).
Runs ON the pc as root. Leaves QEMU running when it returns.
"""
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from w_lib import *  # noqa: E402,F401

PI = load_pi()


def build(net_on=False, headless=True):
    st = json.loads((D / "state.json").read_text())
    PI.validate_state(st)
    assert PI.seed_matches(st)
    cd = [str(D / "media/win.iso"), str(D / "media/virtio-win.iso"), str(D / "media/tools.iso")]
    argv = PI.build_argv(st, "setup", str(D / "image.qcow2"), str(D), qemu=DISTRO_QEMU, cdroms=cd)
    s = " ".join(argv)
    assert "restrict=off" in s
    if not net_on:
        argv = [a.replace("restrict=off", "restrict=on") for a in argv]
    argv = [a.replace("propimage-setup", "propimage-w11-setup") if a == "propimage-setup" else a for a in argv]
    if headless:
        k = argv.index("gtk")
        argv[k] = "none"
    return st, argv


def ocr(png):
    """Text of a screendump (tesseract on the pc). Empty string when it cannot be read."""
    try:
        r = subprocess.run(["tesseract", str(png), "-", "--psm", "11"], capture_output=True, text=True, timeout=60)
        return r.stdout
    except (OSError, subprocess.TimeoutExpired):
        return ""


# One page the answer file does not skip (MEASURED on image B, 2026-11-15: "Product key"; Setup stopped there). The way past it
# without a key: Tab to "I don't have a product key", Enter. Handled by screen text, never by timing; each attempt is logged.


def main():
    attach = "--attach" in sys.argv
    cpu = [r for r in PI.cpu_preflight(json.loads((D / "state.json").read_text()), Path("/proc/cpuinfo").read_text()) if r[1] != "MATCH"]
    if cpu:
        raise SystemExit("CPU_PROFILE_UNSUPPORTED %s" % cpu)
    if attach:
        st, argv = build()
        log("ATTACH to qemu pid %s" % qemu_pid())
    else:
        if qemu_pid():
            raise SystemExit("an win11-base qemu is already running")
        need_free(15, "Windows install growth")
        st, argv = build()
        assert "restrict=on" in " ".join(argv) and "-display none" in " ".join(argv)
        (D / "run/last_cmd.txt").write_text("CMD: " + " ".join(__import__("shlex").quote(a) for a in argv) + "\n")
        start_swtpm(D)
        start_qemu(argv, D)
        log("START setup (headless, restrict=on) qemu pid %s" % qemu_pid())
        (D / "run/marker.txt").open("a").write("START setup-headless %s pid %s\n" % (time.strftime("%Y-%m-%dT%H:%M:%S%z"), qemu_pid()))
    q = None
    for _ in range(40):
        try:
            q = Qmp(D / "qmp.sock")
            break
        except OSError:
            time.sleep(0.5)
    assert q, "no QMP"
    t0 = time.time()
    shots = D / "run/shots"
    shots.mkdir(exist_ok=True)
    sh(["chown", SU, str(shots)])
    n = int(time.time()) % 100000 if attach else 0
    last_shot = 0
    keys_until = 0 if attach else t0 + 40
    key = D / "keys/id_ed25519"
    last_ocr, keypage_tries, seen = 0, 0, set()
    while True:
        now = time.time()
        pid = qemu_pid()
        if not pid:
            log("QEMU GONE after %d s (powered off or crashed)" % (now - t0))
            return 4
        try:
            if now < keys_until:
                q.key("spc")
            if now - last_shot >= 60:
                q.shot(shots / ("s%04d.png" % n))
                n += 1
                last_shot = now
            if now > keys_until and now - last_ocr >= 20:
                last_ocr = now
                q.shot(shots / "ocr.png")
                txt = ocr(shots / "ocr.png").lower()
                words = " ".join(txt.split())[:160]
                if words and words not in seen and len(seen) < 80:
                    seen.add(words)
                    log("screen: %s" % words, "install_screens.log")
                t = txt.replace("\u2019", "'")
                if "enter product key" in t and keypage_tries < 4:   # OCR misses the small link text; the page title and the field label are reliable
                    keypage_tries += 1
                    q.shot(shots / ("keypage_before_%d.png" % keypage_tries))
                    log("product key page detected (attempt %d): Tab x%d, Enter" % (keypage_tries, keypage_tries))
                    for _ in range(keypage_tries):
                        q.key("tab")
                        time.sleep(0.4)
                    q.key("ret")
                    time.sleep(3)
                    q.shot(shots / ("keypage_after_%d.png" % keypage_tries))
        except (OSError, RuntimeError) as e:
            log("qmp: %s" % e)
        if now - t0 > 120 and int(now - t0) % 30 < 3:
            try:
                p = sh(ssh_base(st, key) + ["echo", "up"], check=False, timeout=30)
            except subprocess.TimeoutExpired:
                continue
            if p.returncode == 0 and "up" in p.stdout:
                log("SSH UP after %d s" % (now - t0))
                q.shot(shots / "s_ssh_up.png")
                return 0
        if now - t0 > 6000:
            log("TIMEOUT 100 min without ssh; QEMU left running for inspection")
            return 3
        time.sleep(2)


if __name__ == "__main__":
    sys.exit(main())
