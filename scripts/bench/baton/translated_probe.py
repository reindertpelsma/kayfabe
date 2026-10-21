#!/usr/bin/env python3
"""Read declared Translated guest USERD pairs through the existing read-only probe.

Pending guest work does not prove an idle pump or a GPU stall. Framebuffer
USERDs are explicitly unobserved. This neither reads GP_PUT for scheduling nor
writes/re-rings anything. The caller selects its verified QEMU and RAM split.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import struct
import time


BIRTH = re.compile(r"chan (0x[0-9a-f]+:0x[0-9a-f]+) BORN Translated: token (0x[0-9a-f]+) -> host (0x[0-9a-f]+).*?gpfifo=(0x[0-9a-f]+)x(\d+) userd=Some\((Sysmem|Framebuffer) \{ base: (\d+), size: (\d+) \}\).*?engine=(0x[0-9a-f]+)")
RETIRE = re.compile(r"TSPACE-RETIRE tok=(0x[0-9a-f]+) host=(0x[0-9a-f]+)")


class TranslatedLog:
    def __init__(self):
        self.live = {}

    def line(self, line):
        m = BIRTH.search(line)
        if m:
            key, token, host, fifo, entries, aperture, gpa, size, engine = m.groups()
            host, token = int(host, 16), int(token, 16)
            self.live = {k: v for k, v in self.live.items() if v["host"] != host and k != key}
            self.live[key] = dict(channel=key, token=token, host=host, fifo=int(fifo, 16),
                                  entries=int(entries), aperture=aperture, gpa=int(gpa),
                                  size=int(size), engine=int(engine, 16))
            if len(self.live) > 4096:
                raise RuntimeError("Translated probe channel bound exceeded")
        m = RETIRE.search(line)
        if m:
            token, host = (int(v, 16) for v in m.groups())
            self.live = {k: v for k, v in self.live.items() if (v["token"], v["host"]) != (token, host)}

    def sample(self, read):
        rows = []
        for facts in self.live.values():
            row = dict(facts)
            if facts["aperture"] != "Sysmem":
                row["status"] = "unobserved_framebuffer"
            elif facts["size"] < 512 or facts["gpa"] & 511 or not facts["entries"]:
                row["status"] = "invalid_declared_userd"
            else:
                raw = read(facts["gpa"] + 0x88, 8)  # public NV_RAMUSERD GP_GET / GP_PUT
                if raw is None or len(raw) != 8:
                    row["status"] = "unreadable"
                else:
                    get, put = struct.unpack("<II", raw)
                    row.update(get=get, put=put, delta=(put - get) % facts["entries"],
                               status="observed" if max(get, put) < facts["entries"] else "implausible_indices")
            rows.append(row)
        return rows


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--pid", type=int, required=True)
    ap.add_argument("--log", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--userd-module", type=Path, required=True)
    ap.add_argument("--lowmem", type=lambda x: int(x, 0), required=True)
    ap.add_argument("--seconds", type=int, default=1200)
    args = ap.parse_args()
    spec = importlib.util.spec_from_file_location("userd", args.userd_module)
    userd = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(userd)
    argv = Path(f"/proc/{args.pid}/cmdline").read_bytes().decode().strip("\0").split("\0")
    if "qemu-system" not in argv[0]:
        raise RuntimeError("selected process is not QEMU")
    base, size = userd.guest_ram(args.pid, argv)
    gm = userd.GuestMem(args.pid, base, size, args.lowmem)
    state, tail = TranslatedLog(), ""
    until = time.monotonic() + args.seconds
    try:
        with args.log.open(errors="replace") as log, args.out.open("x", buffering=1) as out:
            while time.monotonic() < until and Path(f"/proc/{args.pid}/cmdline").exists():
                lines = (tail + log.read(4 * 1024 * 1024)).split("\n")
                tail = lines.pop()[-16384:]
                for line in lines:
                    if "BORN Translated:" in line or "TSPACE-RETIRE" in line:
                        state.line(line)
                record = dict(epoch=time.time(), pid=args.pid, ram_base=base, ram_size=size,
                              lowmem=args.lowmem, log_read_bytes=log.tell(), log_size=os.fstat(log.fileno()).st_size,
                              channels=state.sample(gm.read))
                out.write(json.dumps(record) + "\n")
                if out.tell() >= 64 * 1024 * 1024:
                    out.write(json.dumps(dict(limit=True, epoch=time.time())) + "\n")
                    break
                time.sleep(1)
    finally:
        os.close(gm.fd)


if __name__ == "__main__":
    main()
