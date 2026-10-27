"""Small fail-closed guards shared by the Z1 experiment launchers."""

from pathlib import Path


class Refused(ValueError):
    """A real host or privilege precondition did not hold."""


def check_qemus(actual, owned):
    if any(owned.get(pid) != start for pid, start in actual.items()):
        raise Refused("pre-existing/foreign QEMU; leave it untouched")


def check_memory(available_mib, next_guest_mib):
    if available_mib < next_guest_mib + 2048:
        raise Refused("insufficient RAM for next guest plus 2 GiB host headroom")


def check_privilege(uid, effective, permitted, groups, kvm_gid, primary_gid):
    if uid == 0 or effective != 0 or permitted != 0:
        raise Refused("rootless launch requires non-root with CapEff=CapPrm=0")
    if kvm_gid not in groups or groups - {kvm_gid, primary_gid}:
        raise Refused("only the primary group and kvm are permitted")


def status(pid):
    return dict(line.split(":", 1) for line in Path(f"/proc/{pid}/status").read_text().splitlines())


def identity(pid):
    return int(Path(f"/proc/{pid}/stat").read_text().rpartition(")")[2].split()[19])


def qemus():
    result = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if (entry / "comm").read_text().strip().startswith("qemu-system"):
                result[int(entry.name)] = identity(int(entry.name))
        except (FileNotFoundError, ProcessLookupError):
            continue
    return result


def preflight(ram_mib=2048, owned=None):
    check_qemus(qemus(), owned or {})
    fields = Path("/proc/meminfo").read_text().splitlines()
    available = next(
        int(line.split()[1]) // 1024 for line in fields if line.startswith("MemAvailable:")
    )
    check_memory(available, ram_mib)
    if not Path("/dev/null").is_char_device():
        raise Refused("/dev/null is not a character device")
    return {"available_mib": available, "guest_mib": ram_mib, "owned": owned or {}}
