# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Pinned official Zig toolchain bootstrap and deterministic Windows probe builds.

Only the compiler distribution is unpacked on the host, never an app installer.
All processes are direct executable/argument-list invocations, with shell=False.
"""
import os
from pathlib import Path
import platform
import shutil
import struct
import subprocess
import tarfile
import zipfile

TOOL_NAMES = ("cup2.exe", "cup3.exe", "cup8.exe", "kf_cuda_control.exe",
              "kf_dxprobe.exe", "kf_glgears.exe", "kf_overlayprobe.exe")
NOTICE_SOURCES = ("LICENSE", "lib/libcxx/LICENSE.TXT", "lib/libcxxabi/LICENSE.TXT",
                  "lib/libunwind/LICENSE.TXT", "lib/libc/mingw/COPYING")
NOTICE_NAMES = tuple("licenses/zig/" + p for p in NOTICE_SOURCES)
HOSTS = frozenset(f"{arch}-{system}" for arch in ("x86_64", "aarch64") for system in ("linux", "macos", "windows"))
HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
SOURCES = ("scripts/bench/windows/appmatrix/tools/src/kf_dxprobe.cpp",
           "scripts/bench/windows/appmatrix/tools/src/kf_overlayprobe.cpp",
           "scripts/bench/windows/appmatrix/tools/src/kf_overlayprobe_logic.h",
           "scripts/bench/windows/appmatrix/tools/src/kf_glgears.c",
           "scripts/bench/windows/appmatrix/tools/src/kf_cuda_control.c",
           "scripts/bench/windows/cuda/nvcuda.def", "scripts/bench/cuda_min/cuda.h",
           "archive/nvkvm/tests/mode2/cup2.c", "scripts/bench/cup3.c", "scripts/bench/cup8.c")


def check_toolchain(toolchain):
    from package_bundle import validate_pin
    import bundle_repro
    if toolchain.get("name") != "zig" or toolchain.get("version") != "0.13.0" or set(toolchain.get("hosts", {})) != HOSTS:
        raise ValueError("toolchain must pin official Zig 0.13.0 for all six supported hosts")
    for host, pin in toolchain["hosts"].items():
        validate_pin("zig-" + host, pin)
        if pin["url"] != "https://ziglang.org/download/0.13.0/" + pin["name"]:
            raise ValueError("toolchain URL is not the official pinned release: " + host)
    for name in toolchain.get("programs", {}):
        bundle_repro.policy("tools/" + name, toolchain)
    return toolchain


def host_pin(toolchain, system=None, machine=None):
    system = (system or platform.system()).lower()
    system = {"darwin": "macos"}.get(system, system)
    machine = (machine or platform.machine()).lower()
    machine = {"amd64": "x86_64", "arm64": "aarch64"}.get(machine, machine)
    host = machine + "-" + system
    if host not in toolchain["hosts"]:
        raise ValueError("unsupported packaging host: " + host + "; use an x86_64 or aarch64 Linux, macOS or Windows host")
    return host, toolchain["hosts"][host]


def unpack_compiler(archive, pin, destination):
    """Fresh, validated, regular-file-only compiler extraction; portable to Windows.

    No reliance on tar/unzip commands, host archive programs, archive ownership or
    permissions. Never reuse a previously unpacked (possibly changed) compiler.
    """
    from package_bundle import verify_file
    verify_file("zig", pin, archive)
    destination = Path(destination)
    if destination.exists():
        raise ValueError("compiler extraction requires an empty destination")
    destination.mkdir(parents=True)
    seen = set()
    def target(name):
        parts = name.rstrip("/").split("/")
        if any(p in ("", ".", "..") or ":" in p or "\\" in p for p in parts):
            raise ValueError("unsafe compiler archive path")
        key = "/".join(parts).casefold()
        if key in seen:
            raise ValueError("duplicate compiler archive member")
        seen.add(key)
        return destination.joinpath(*parts)
    try:
        if pin["name"].endswith(".zip"):
            with zipfile.ZipFile(archive) as z:
                for member in z.infolist():
                    dst = target(member.filename)
                    kind = (member.external_attr >> 16) & 0o170000
                    if kind not in (0, 0o040000, 0o100000):
                        raise ValueError("compiler archive contains link/special file")
                    if member.is_dir():
                        dst.mkdir(parents=True, exist_ok=True)
                    else:
                        dst.parent.mkdir(parents=True, exist_ok=True)
                        with z.open(member) as source, dst.open("wb") as out:
                            shutil.copyfileobj(source, out, 1 << 20)
        else:
            with tarfile.open(archive, "r:xz") as tar:
                for member in tar:
                    dst = target(member.name)
                    if member.isdir():
                        dst.mkdir(parents=True, exist_ok=True)
                    elif member.isfile():
                        dst.parent.mkdir(parents=True, exist_ok=True)
                        with tar.extractfile(member) as source, dst.open("wb") as out:
                            shutil.copyfileobj(source, out, 1 << 20)
                    else:
                        raise ValueError("compiler archive contains link/special file")
        executable = destination / pin["name"].removesuffix(".tar.xz").removesuffix(".zip") / ("zig.exe" if pin["name"].endswith(".zip") else "zig")
        if not executable.is_file():
            raise ValueError("compiler executable missing from official archive")
        executable.chmod(0o755)
        return executable
    except BaseException:
        shutil.rmtree(destination)
        raise


def build_commands(zig, source_root):
    """Arguments are relative to a canonical LF-only source tree and fixed outputs."""
    base = [str(zig), "cc", "-target", "x86_64-windows-gnu", "-mcpu=baseline", "-O2", "-g0", "-s", "-fno-ident",
            "-ffile-prefix-map=" + str(source_root) + "=/kf-src", "-fdebug-prefix-map=" + str(source_root) + "=/kf-src"]
    am = "scripts/bench/windows/appmatrix/tools/src/"
    cpp = base.copy(); cpp[1] = "c++"; cpp += ["-std=gnu++17", "-fms-extensions", "-static"]
    cuda = base + ["-I", "scripts/bench/cuda_min", "nvcuda.lib"]
    commands = [[str(zig), "dlltool", "-m", "i386:x86-64", "-d", "scripts/bench/windows/cuda/nvcuda.def", "-l", "nvcuda.lib", "-D", "nvcuda.dll"]]
    # Same SDK constant fallback already used by windows_gate.sh for older headers.
    # Microsoft Learn d3dcommon/D3D_FEATURE_LEVEL documents 12_2 = 0xc200.
    commands += [cpp + ["-DD3D_FEATURE_LEVEL_12_2=((D3D_FEATURE_LEVEL)0xc200)", am + "kf_dxprobe.cpp", "-o", "kf_dxprobe.exe", "-ld3d11", "-ld3d12", "-ldxgi", "-ldxguid"],
                 cpp + [am + "kf_overlayprobe.cpp", "-o", "kf_overlayprobe.exe", "-ld3d11", "-ldxgi", "-ldcomp", "-ldxguid", "-luuid", "-lwinmm", "-luser32", "-lgdi32"],
                 base + [am + "kf_glgears.c", "-o", "kf_glgears.exe", "-lopengl32", "-lgdi32", "-lm"]]
    commands += [cuda + [source, "-o", name] for name, source in (
        ("cup2.exe", "archive/nvkvm/tests/mode2/cup2.c"),
        ("cup3.exe", "scripts/bench/cup3.c"), ("cup8.exe", "scripts/bench/cup8.c"),
        ("kf_cuda_control.exe", am + "kf_cuda_control.c"))]
    return commands


def copy_notices(compiler_root, source_root):
    """Ship the compiler/runtime notices with our compiled probes (LF canonical)."""
    files = {}
    for rel, name in zip(NOTICE_SOURCES, NOTICE_NAMES):
        data = (Path(compiler_root) / rel).read_bytes().replace(b"\r\n", b"\n")
        dest = Path(source_root) / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(data)
        files[name] = dest
    return files


def pe_info(path):
    """Read PE/COFF timestamp, machine and import descriptors without executing it."""
    data = Path(path).read_bytes()
    if len(data) < 64 or data[:2] != b"MZ":
        raise ValueError("probe is not PE: " + Path(path).name)
    pe = struct.unpack_from("<I", data, 60)[0]
    if pe + 264 > len(data) or data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("invalid PE header")
    machine, nsections, timestamp = struct.unpack_from("<HHI", data, pe + 4)
    optsize = struct.unpack_from("<H", data, pe + 20)[0]
    if machine != 0x8664 or struct.unpack_from("<H", data, pe + 24)[0] != 0x20B:
        raise ValueError("probe is not x86_64 PE32+")
    sections = pe + 24 + optsize
    def offset(rva):
        for i in range(nsections):
            at = sections + 40 * i
            virtualsize, address, rawsize, raw = struct.unpack_from("<IIII", data, at + 8)
            if address <= rva < address + max(virtualsize, rawsize):
                off = rva - address + raw
                if not 0 <= off < len(data):
                    break
                return off
        raise ValueError("invalid PE RVA")
    imports = []
    rva, size = struct.unpack_from("<II", data, pe + 24 + 120)
    if rva:
        start = offset(rva)
        for pos in range(start, min(start + size, len(data) - 19), 20):
            desc = struct.unpack_from("<IIIII", data, pos)
            if not any(desc):
                break
            name = offset(desc[3])
            imports.append(data[name:data.index(b"\0", name)].decode("ascii").lower())
    return dict(machine=machine, timestamp=timestamp, imports=sorted(imports))


def build_tools(toolchain, archive, output, repo=REPO):
    from package_bundle import canonical, sha256
    check_toolchain(toolchain)
    _, pin = host_pin(toolchain)
    output = Path(output).resolve()
    if output.exists():
        raise ValueError("probe build requires an empty output directory: " + str(output))
    output.mkdir(parents=True)
    try:
        zig = unpack_compiler(archive, pin, output / "compiler")
        source_root = output / "src"
        source_hashes = {}
        for rel in SOURCES:
            source = Path(repo) / rel
            if source.is_symlink():
                raise ValueError("probe source link refused: " + rel)
            data = source.read_bytes().replace(b"\r\n", b"\n")
            dest = source_root / rel
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(data)
            source_hashes[rel] = sha256(dest)
        env = dict(os.environ)
        # Zig/Clang implicit paths and caches are explicit. No shell/CFLAGS parsing.
        for key in ("CPATH", "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH", "LIBRARY_PATH", "SDKROOT", "MACOSX_DEPLOYMENT_TARGET", "ZIG_LIB_DIR", "ZIG_LIBC"):
            env.pop(key, None)
        # Zig's COFF linker copies SOURCE_DATE_EPOCH into TimeDateStamp. Zero
        # removes it; the ISO's separate fixed recording date remains 2000-01-01.
        env.update(SOURCE_DATE_EPOCH="0", TZ="UTC", LC_ALL="C", NO_COLOR="1",
                   ZIG_GLOBAL_CACHE_DIR=str(output / "cache-global"), ZIG_LOCAL_CACHE_DIR=str(output / "cache-local"))
        for command in build_commands(zig, source_root):
            name = command[command.index("-o") + 1] if "-o" in command else "nvcuda import library"
            print("BUILD_TOOL " + name, flush=True)
            result = subprocess.run(command, cwd=source_root, env=env, shell=False, stdin=subprocess.DEVNULL,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=1200)
            if result.returncode:
                raise ValueError(f"probe {name}: pinned zig failed rc={result.returncode}\n" + result.stdout.decode("utf-8", "replace")[-6000:])
        tools = {name: source_root / name for name in TOOL_NAMES}
        for name, path in tools.items():
            info = pe_info(path)
            if info["timestamp"] != 0:
                raise ValueError("probe PE timestamp is not zero: " + name)
            if any(x in info["imports"] for x in ("libgcc_s_seh-1.dll", "libstdc++-6.dll", "libc++.dll", "libwinpthread-1.dll")):
                raise ValueError("probe has non-Windows runtime dependency: " + name)
            if str(output).encode() in path.read_bytes() or str(output).encode("utf-16-le") in path.read_bytes():
                raise ValueError("probe leaks build path: " + name)
        notices = copy_notices(zig.parent, source_root)
        (output / "tools.receipt.json").write_bytes(canonical(dict(sources=source_hashes,
            files={name: dict(sha256=sha256(path), pe=pe_info(path)) for name, path in tools.items()},
            notices={name: sha256(path) for name, path in notices.items()},
            target="x86_64-windows-gnu", toolchain=toolchain)))
        return dict(tools, **notices)
    finally:
        # Retain only the small compiled tools and receipt until packaging ends.
        for name in ("compiler", "cache-global", "cache-local"):
            shutil.rmtree(output / name, ignore_errors=True)
