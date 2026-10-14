#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Read-only and no-board-data guard for tools/windows-capture.

Fails (exit 1) when the driver, the library or the companion contains anything that could store
to device memory, device registers, PCI configuration space or foreign memory, hook or patch
anything, or touch board-specific data. Also checks a built kfwc.sys's import table when given.

    guard_readonly.py [--root DIR] [--sys kfwc.sys] [--self-test]

--self-test injects known violations into a scratch copy and requires the guard to catch each
one (a guard that cannot fail proves nothing), then checks that the real tree is clean.
Layers it enforces (see lib/kfwc_platform.h):
  1 type    kfwc_platform has no store method; the mapped pointer is KFWC_MMIO (pointer to
            volatile const), and every line that names it matches a short allow-list;
  2 source  forbidden APIs/patterns below;
  3 binary  the import table of kfwc.sys.
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_ROOT = os.path.dirname(HERE)

# ---- source-level rules ---------------------------------------------------------------------
FORBIDDEN_DRIVER = [
    (r"\bWRITE_REGISTER_\w+", "store to a device register"),
    (r"\bWRITE_PORT_\w+", "store to an I/O port"),
    (r"\bHalSetBusData\w*", "store to PCI configuration space"),
    (r"\bHalAssignSlotResources\b", "claims device resources"),
    (r"\bMmMapIoSpace\s*\(", "use MmMapIoSpaceEx with PAGE_READONLY"),
    (r"\bPAGE_READWRITE\b|\bPAGE_WRITECOPY\b|\bPAGE_EXECUTE\w*", "writable or executable mapping"),
    (r"\bMmMapLockedPages\w*", "maps memory writable"),
    (r"\bMmGetVirtualForPhysical\b|\bZwMapViewOfSection\b|\bZwOpenSection\b", "maps physical memory"),
    (r"MM_COPY_MEMORY_VIRTUAL", "only physical copies are allowed (RAM ranges are checked first)"),
    (r"\bPsSetCreateProcessNotify\w*|\bPsSetLoadImageNotifyRoutine\w*|\bObRegisterCallbacks\b|\bCmRegisterCallback\w*",
     "hooking"),
    (r"\bIoConnectInterrupt\w*|\bIoAttachDevice\w*|\bIoRegisterPlugPlayNotification\b", "attaches to or hooks a device"),
    (r"\b__writemsr\b|\b__outbyte\b|\b__outword\b|\b__outdword\b|\b__writecr\d\b|\b__writedr\b", "privileged store"),
    (r"\bMmGetSystemRoutineAddress\b|\bKeServiceDescriptorTable\b", "resolves/patches system routines"),
    (r"\bIoCreateDevice\s*\(", "use IoCreateDeviceSecure (SYSTEM and Administrators only)"),
    (r"\bMmCopyMemory\b(?![^;]*MM_COPY_MEMORY_PHYSICAL)", "MmCopyMemory must be MM_COPY_MEMORY_PHYSICAL"),
]
# user-mode source and the sidecar writer must not name any board or person identifying datum
BANNED_USER = re.compile(
    r"SUBSYS|SerialNumber|\bUUID\b|\bGUID_DEVINTERFACE|MatchingDeviceId|HardwareID|\bVBIOS\b|\bMAC[_ ]?ADDR|"
    r"GetComputerName|GetUserName|gethostname|\bUSERNAME\b|\bCOMPUTERNAME\b|SetupDi", re.I)
BANNED_USER_FILES = ("user/kfwc_collect.c", "lib/kfwc_meta.c")

# lines that may mention the mapped pointer; everything else naming it is a violation
MMIO_ALLOWED = [
    r"^\s*KFWC_MMIO bar0;",
    r"^\s*PVOID bar0_map;",
    r"^\s*if \(s->bar0_map\) \{$",
    r"^\s*MmUnmapIoSpace\(s->bar0_map, KFWC_BAR0_MAP_BYTES\);$",
    r"^\s*s->bar0_map = NULL;$",
    r"^\s*s->bar0 = NULL;$",
    r"^\s*s->bar0_map = MmMapIoSpaceEx\(phys, KFWC_BAR0_MAP_BYTES, PAGE_READONLY \| PAGE_NOCACHE\);$",
    r"^\s*s->bar0 = \(KFWC_MMIO\)s->bar0_map;$",
    r"^\s*if \(s->bar0 && s->bar0_pa == pa\) \{$",
    r"^\s*if \(!s->bar0\) \{$",
    r"^\s*if \(!kfwc_reg_allowed\(off\).*!bar_ready\(s\)\)$",
    r"^\s*\*out = \*\(const volatile ULONG \*\)\(s->bar0 \+ off\);$",
    r"^\s*s->bar0_pa = (0|pa);$",
    r"^\s*s->bar0_pa = pa;$",
    r"^\s*ULONGLONG bar0_pa;.*$",
    r"^\s*static BOOLEAN pci_bar0\(.*$",
    r"^\s*if \(!pci_bar0\(g, &pa\)\) \{$",
    r"^\s*(//|/\*|\*).*$",
]
MMIO_NAME = re.compile(r"\bbar0(_map)?\b")

# the platform struct: only these members, and nothing that looks like a store
PLATFORM_MEMBERS = {"ctx", "read_ram", "ram_contains", "ram_range_count", "ram_range", "read_reg32", "now"}


def strip_comments(text):
    text = re.sub(r"/\*.*?\*/", lambda m: "\n" * m.group(0).count("\n"), text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def source_files(root, sub, exts):
    out = []
    for d in sub:
        for dp, _, fs in os.walk(os.path.join(root, d)):
            for f in fs:
                if f.endswith(exts):
                    out.append(os.path.join(dp, f))
    return sorted(out)


def check(root, sys_path=None):
    problems = []

    def add(path, line, msg):
        problems.append("%s:%d: %s" % (os.path.relpath(path, root), line, msg))

    for path in source_files(root, ["driver", "lib"], (".c", ".h")):
        raw = open(path, encoding="utf-8").read()
        code = strip_comments(raw)
        for n, line in enumerate(code.split("\n"), 1):
            for pat, why in FORBIDDEN_DRIVER:
                if re.search(pat, line):
                    add(path, n, why)
        # every MmMapIoSpaceEx statement must say PAGE_READONLY
        for m in re.finditer(r"MmMapIoSpaceEx\s*\(([^;]*);", code, flags=re.S):
            if "PAGE_READONLY" not in m.group(1):
                add(path, code[: m.start()].count("\n") + 1, "MmMapIoSpaceEx without PAGE_READONLY")
        # the mapped pointer may only appear in allow-listed forms
        for n, line in enumerate(raw.split("\n") if os.sep + "driver" + os.sep in path else [], 1):
            if MMIO_NAME.search(line) and not any(re.match(p, line) for p in MMIO_ALLOWED):
                add(path, n, "unexpected use of the mapped device pointer: %s" % line.strip())
    # the platform interface has no store
    plat = os.path.join(root, "lib", "kfwc_platform.h")
    if os.path.exists(plat):
        text = strip_comments(open(plat, encoding="utf-8").read())
        m = re.search(r"typedef struct kfwc_platform \{(.*?)\} kfwc_platform;", text, flags=re.S)
        if not m:
            add(plat, 1, "kfwc_platform definition not found")
        else:
            members = set()
            for decl in m.group(1).split(";"):
                decl = decl.strip()
                if not decl:
                    continue
                fn = re.search(r"\(\s*\*\s*(\w+)\s*\)", decl)
                members.add(fn.group(1) if fn else decl.split()[-1].lstrip("*"))
            if members != PLATFORM_MEMBERS:
                add(plat, 1, "kfwc_platform members changed: %s" % sorted(members ^ PLATFORM_MEMBERS))
            for mem in members:
                if re.search(r"write|store|put|set|poke|out", mem, re.I):
                    add(plat, 1, "store-like platform member %s" % mem)
    # user mode: no board/person identifying reads
    for rel in BANNED_USER_FILES:
        path = os.path.join(root, rel)
        if not os.path.exists(path):
            continue
        for n, line in enumerate(strip_comments(open(path, encoding="utf-8").read()).split("\n"), 1):
            m = BANNED_USER.search(line)
            if m:
                add(path, n, "board/person identifying word: %s" % m.group(0))
    # the build must keep const-correctness enforcement on
    b = os.path.join(root, "build", "build-linux.sh")
    if os.path.exists(b) and "-Wcast-qual" not in open(b, encoding="utf-8").read():
        add(b, 1, "driver build lost -Wcast-qual (the const-MMIO type discipline depends on it)")
    # binary imports
    if sys_path:
        out = ""
        for tool in ("llvm-readobj-19", "llvm-readobj", "llvm-readobj-21"):
            if shutil.which(tool):
                out = subprocess.run([tool, "--coff-imports", sys_path], capture_output=True, text=True).stdout
                break
        syms = set(re.findall(r"Symbol: (\w+)", out))
        if not syms:
            add(sys_path, 0, "cannot read the import table (llvm-readobj missing?)")
        for s in sorted(syms):
            if re.match(r"(WRITE_|HalSetBusData|MmMapIoSpace$|MmMapLockedPages|ZwMapViewOfSection|MmGetVirtualForPhysical|"
                        r"PsSetCreateProcess|PsSetLoadImage|ObRegisterCallbacks|CmRegisterCallback|IoAttachDevice)", s):
                add(sys_path, 0, "forbidden import %s" % s)
        if syms and "MmMapIoSpaceEx" not in syms:
            add(sys_path, 0, "MmMapIoSpaceEx missing from imports: BAR0 mapping changed?")
    return problems


# ---- self test ---------------------------------------------------------------------------------
INJECT = [
    ("driver/kfwc_drv.c", "WRITE_REGISTER_ULONG((PULONG)0, 1);", "store to a device register"),
    ("driver/kfwc_drv.c", "HalSetBusDataByOffset(PCIConfiguration, 0, 0, 0, 4, 4);", "store to PCI configuration space"),
    ("driver/kfwc_drv.c", "ULONG x = PAGE_READWRITE;", "writable or executable mapping"),
    ("driver/kfwc_drv.c", "MmMapIoSpace(phys, 4096, MmNonCached);", "use MmMapIoSpaceEx"),
    ("driver/kfwc_drv.c", "void *m = MmMapIoSpaceEx(phys, 4096, PAGE_NOCACHE);", "PAGE_READONLY"),
    ("driver/kfwc_drv.c", "*(volatile ULONG *)(s->bar0 + 0x110040) = 0;", "mapped device pointer"),
    ("driver/kfwc_drv.c", "((volatile UCHAR *)s->bar0)[0] = 1;", "mapped device pointer"),
    ("driver/kfwc_drv.c", "ObRegisterCallbacks(0, 0);", "hooking"),
    ("driver/kfwc_drv.c", "MmCopyMemory(a, b, 8, MM_COPY_MEMORY_VIRTUAL, &c);", "only physical copies are allowed"),
    ("lib/kfwc_platform.h", None, "store-like platform member"),   # handled below
    ("user/kfwc_collect.c", "const char *x = \"SUBSYS_\";", "board/person"),
    ("user/kfwc_collect.c", "GetComputerNameA(0, 0);", "board/person"),
]


def self_test():
    ok = True
    base = check(DEFAULT_ROOT)
    if base:
        print("self-test: the real tree is NOT clean:")
        print("\n".join(base))
        return 1
    for rel, line, expect in INJECT:
        with tempfile.TemporaryDirectory() as tmp:
            for d in ("driver", "lib", "user", "build"):
                shutil.copytree(os.path.join(DEFAULT_ROOT, d), os.path.join(tmp, d))
            target = os.path.join(tmp, rel)
            text = open(target, encoding="utf-8").read()
            if line is None:  # add a store-like member to the platform struct
                text = text.replace("kfwc_u64 (*now)(void *ctx);", "kfwc_u64 (*now)(void *ctx);\n    int (*write_reg32)(void *ctx, kfwc_u32 off, kfwc_u32 v);")
            else:
                text += "\nstatic void injected(void) { %s }\n" % line
            open(target, "w", encoding="utf-8").write(text)
            found = check(tmp)
            hit = any(expect in p for p in found)
            print("self-test %-52s %s" % (expect + " in " + rel, "caught" if hit else "MISSED"))
            ok = ok and hit
    print("guard self-test: %s" % ("all injected violations caught, real tree clean" if ok else "FAILED"))
    return 0 if ok else 1


def main(argv):
    root, sys_path = DEFAULT_ROOT, None
    i = 0
    while i < len(argv):
        if argv[i] == "--self-test":
            return self_test()
        if argv[i] == "--root":
            root = argv[i + 1]
            i += 1
        elif argv[i] == "--sys":
            sys_path = argv[i + 1]
            i += 1
        else:
            print(__doc__)
            return 2
        i += 1
    problems = check(root, sys_path)
    for p in problems:
        print("GUARD VIOLATION: " + p)
    print("guard_readonly: %s" % ("FAILED (%d)" % len(problems) if problems else "clean"))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
