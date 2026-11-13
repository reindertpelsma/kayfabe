"""Measure capabilities. A field that cannot be measured is None; nothing is guessed."""

import os
import platform
import re
import shutil
import subprocess


class Probes:
    """All host access in one injectable object (tests subclass or replace the methods)."""

    def run(self, argv, timeout=10):
        try:
            done = subprocess.run(argv, capture_output=True, text=True, timeout=timeout, check=False)
        except (OSError, subprocess.SubprocessError):
            return None
        return done.stdout if done.returncode == 0 else None

    def read(self, path, limit=1 << 20):
        try:
            with open(path, encoding="utf-8", errors="replace") as stream:
                return stream.read(limit)
        except OSError:
            return None

    def exists(self, path):
        return os.path.exists(path)

    def access_rw(self, path):
        return os.access(path, os.R_OK | os.W_OK)

    def listdir(self, path):
        try:
            return sorted(os.listdir(path))
        except OSError:
            return []

    def env(self, name):
        return os.environ.get(name)

    def euid(self):
        return os.geteuid()

    def cpu_count(self):
        return os.cpu_count()

    def disk_free(self, path):
        try:
            return shutil.disk_usage(path).free
        except OSError:
            return None

    def kernel(self):
        return platform.release() or None


def gpu(p):
    out = p.run(
        ["nvidia-smi", "--query-gpu=name,compute_cap,memory.total,driver_version,pci.device_id",
         "--format=csv,noheader,nounits"]
    )
    blank = dict(gpu_name=None, compute_cap=None, vram_gb=None, driver=None, pci_device_id=None)
    if not out or not out.strip():
        return blank
    parts = [x.strip() for x in out.strip().splitlines()[0].split(",")]
    if len(parts) != 5:
        return blank
    name, cap, mem, drv, pci = parts
    # compute_cap is the string nvidia-smi prints ("8.9"), as the Worker's hello validator requires.
    cap = cap if re.fullmatch(r"[0-9]{1,2}\.[0-9]", cap) else None
    try:
        vram = round(float(mem) / 1024, 1)
    except ValueError:
        vram = None
    # pci.device_id prints 0xDDDDVVVV (device then vendor); keep the 4-hex device id, lowercase.
    m = re.fullmatch(r"0x([0-9A-Fa-f]{4})[0-9A-Fa-f]{4}", pci)
    return dict(gpu_name=name or None, compute_cap=cap, vram_gb=vram, driver=drv or None,
                pci_device_id=m.group(1).lower() if m else None)


def module_flavor(p):
    text = p.read("/proc/driver/nvidia/version")
    if not text:
        return None
    if "Open Kernel Module" in text:
        return "open"
    if "Kernel Module" in text:
        return "proprietary"
    return None


def in_container(p):
    if p.exists("/.dockerenv") or p.exists("/run/.containerenv") or p.env("container"):
        return True
    cg = p.read("/proc/1/cgroup")
    if cg is None:
        return None
    return bool(re.search(r"docker|kubepods|containerd|lxc|libpod", cg))


def display_present(p):
    if p.env("DISPLAY") or p.env("WAYLAND_DISPLAY"):
        return True
    for card in p.listdir("/sys/class/drm"):
        if (p.read("/sys/class/drm/%s/status" % card, 32) or "").strip() == "connected":
            return True
    return False


def meminfo_gb(p):
    m = re.search(r"MemTotal:\s+(\d+) kB", p.read("/proc/meminfo") or "")
    return round(int(m.group(1)) / 1048576, 1) if m else None


def cpu_model(p):
    m = re.search(r"^model name\s*:\s*(.+)$", p.read("/proc/cpuinfo") or "", re.M)
    return m.group(1).strip() if m else None


def os_name(p):
    m = re.search(r'^PRETTY_NAME="?([^"\n]+)"?', p.read("/etc/os-release") or "", re.M)
    return m.group(1) if m else None


def measure(p, work_dir):
    container = in_container(p)
    free = p.disk_free(work_dir)
    kvm = p.exists("/dev/kvm")
    caps = dict(gpu(p))
    caps.update(
        module_flavor=module_flavor(p),
        kvm=p.access_rw("/dev/kvm") if kvm else False,
        cpu_model=cpu_model(p),
        cores=p.cpu_count(),
        ram_gb=meminfo_gb(p),
        disk_free_gb=round(free / 2**30, 1) if free is not None else None,
        display=display_present(p),
        container=container,
        can_swap_driver=(p.euid() == 0 and container is False) if container is not None else None,
        os=os_name(p),
        kernel=p.kernel(),
    )
    return caps
