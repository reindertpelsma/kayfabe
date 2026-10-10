#!/usr/bin/env python3
"""Read guest-physical bytes from a QEMU dump-guest-memory ELF (paging=false). usage: elfpa.py dump.elf GPA LEN"""
import struct, sys
path, gpa, ln = sys.argv[1], int(sys.argv[2], 16), int(sys.argv[3], 0)
with open(path, "rb") as f:
    h = f.read(64)
    phoff, = struct.unpack_from("<Q", h, 0x20); phentsize, phnum = struct.unpack_from("<HH", h, 0x36)
    f.seek(phoff); ph = f.read(phentsize * phnum)
    for i in range(phnum):
        typ, flags, off, vaddr, paddr, filesz, memsz, align = struct.unpack_from("<IIQQQQQQ", ph, i * phentsize)
        if typ == 1 and paddr <= gpa < paddr + filesz:
            f.seek(off + gpa - paddr); data = f.read(ln)
            for o in range(0, len(data), 16):
                w = struct.unpack_from("<4I", data, o) if o + 16 <= len(data) else ()
                print(f"{gpa + o:#x}: " + " ".join(f"{x:08x}" for x in w))
            sys.exit(0)
print("GPA not in dump")
