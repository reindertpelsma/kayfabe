#!/usr/bin/env python3
"""Read pinned NVIDIA Linux installer metadata; never execute the installer.

Research only. RS_RESOURCE_DESC and NVOC_EXPORTED_METHOD_DEF field layouts
come from the corresponding public OGKM headers. ELF offsets are section
relative, not runtime addresses. Only reviewed, hash-pinned installers enter
the parser. Output contains derived metadata, not executable bytes.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import tarfile
import tempfile


INSTALLERS = {
    "535.309.01": "288b4902ea79b017b49a9226a84f7eaa3e6b49ca0146ae50277c2fe19dd39803",
    "610.43.02": "3034a054bb4cdf7752ff8dc272564cb105513804bff53538945901b16ca77463",
}
PUBLIC_COMMITS = {
    "535.309.01": "15b1a21dfe4eab54a463a0ea3086823ae35fe02e",
    "610.43.02": "86856f779971146f807a7ae2a864b4aee147b553",
}
INCLUDES = ["src/common/sdk/nvidia/inc", "src/common/inc", "src/nvidia/generated",
            "src/nvidia/inc", "src/nvidia/inc/kernel", "src/nvidia/inc/libraries",
            "src/nvidia/src/kernel/rmapi", "src/common/nvpu/inc",
            "src/common/shared/msgq/inc", "src/common/uproc/os/common/include",
            "src/common/unix/common/inc"]


def compiled_metadata(git_root, version):
    """Compile offsetof/sizeof from matching public headers, strings disabled."""
    commit = PUBLIC_COMMITS[version]
    with tempfile.TemporaryDirectory(prefix="kf-runlist-metadata-") as work:
        root = Path(work)
        with subprocess.Popen(["git", "-C", str(git_root), "archive", commit,
                               "src/common", "src/nvidia/inc", "src/nvidia/generated",
                               "src/nvidia/src/kernel/rmapi"], stdout=subprocess.PIPE) as archive:
            subprocess.run(["tar", "-x", "-C", str(root)], stdin=archive.stdout, check=True)
            if archive.wait() != 0:
                raise ValueError("public git archive failed")
        probe = Path(__file__).with_name("runlist-metadata-layout.c")
        subprocess.run(["cc", "-m64", "-DNV_PRINTF_STRINGS_ALLOWED=0",
                        *["-I" + str(root / p) for p in INCLUDES], str(probe),
                        "-o", str(root / "probe")], check=True)
        layouts = json.loads(subprocess.check_output([str(root / "probe")], text=True))
    return {"public_commit": commit, "compiler": subprocess.check_output(
        ["cc", "--version"], text=True).splitlines()[0],
        "mode": "x86_64; NV_PRINTF_STRINGS_ALLOWED=0", "layouts": layouts}


def cstring(data, offset):
    return data[offset:data.index(b"\0", offset)].decode("ascii")


class Elf:
    def __init__(self, data, layout):
        if data[:6] != b"\x7fELF\x02\x01":
            raise ValueError("expected ELF64 little endian")
        self.data = data
        self.layout = layout
        start = struct.unpack_from("<Q", data, 40)[0]
        width, count, name_index = struct.unpack_from("<HHH", data, 58)
        raw = [struct.unpack_from("<IIQQQQIIQQ", data, start + i * width)
               for i in range(count)]
        names = data[raw[name_index][4]:raw[name_index][4] + raw[name_index][5]]
        self.sections = [
            {"name": cstring(names, h[0]), "type": h[1], "offset": h[4],
             "size": h[5], "link": h[6], "info": h[7], "entry_size": h[9]}
            for h in raw
        ]
        self.symbols = []
        self.relocations = {}
        for section in self.sections:
            if section["type"] != 2:
                continue
            strings = self.bytes(section["link"])
            for pos in range(0, section["size"], section["entry_size"]):
                name, info, _, index, value, size = struct.unpack_from(
                    "<IBBHQQ", data, section["offset"] + pos)
                self.symbols.append({"name": cstring(strings, name), "type": info & 15,
                                     "section": index, "offset": value, "size": size})
        for section in self.sections:
            if section["type"] != 4:
                continue
            for pos in range(0, section["size"], section["entry_size"]):
                offset, info, addend = struct.unpack_from(
                    "<QQq", data, section["offset"] + pos)
                self.relocations[section["info"], offset] = (info & 0xffffffff,
                                                             info >> 32, addend)

    def index(self, name):
        return next(i for i, s in enumerate(self.sections) if s["name"] == name)

    def bytes(self, index):
        section = self.sections[index]
        return self.data[section["offset"]:section["offset"] + section["size"]]

    def pointer(self, section, offset):
        kind, symbol_index, addend = self.relocations[section, offset]
        if kind != 1:
            raise ValueError("expected R_X86_64_64 pointer relocation")
        symbol = self.symbols[symbol_index]
        return {"section": self.sections[symbol["section"]]["name"],
                "section_index": symbol["section"],
                "offset": symbol["offset"] + addend,
                "symbol": symbol["name"], "symbol_size": symbol["size"]}

    def class_info(self, pointer):
        layout = self.layout["NVOC_CLASS_INFO"]
        data = self.bytes(pointer["section_index"])
        size = struct.unpack_from("<I", data, pointer["offset"] + layout["size"])[0]
        # 610 uses a bit-field; the public compiler probe supplies its bit mask.
        mask = int.from_bytes(bytes(layout["classIdProbe"]), "little")
        bits = int.from_bytes(data[pointer["offset"]:pointer["offset"] + layout["sizeof"]],
                              "little")
        class_id = (bits & mask) // (mask & -mask)
        return {**pointer, "object_size": size, "class_id": hex(class_id)}

    def inspect(self):
        result = {"sha256": hashlib.sha256(self.data).hexdigest(),
                  "bytes": len(self.data), "class_b297": [], "controls": []}
        section = self.index(".data")
        data = self.bytes(section)
        row = self.layout["RS_RESOURCE_DESC"]
        if row["externalClassId"] != 0 or row["internalClassId"] != 4:
            raise ValueError("reviewed adjacent resource class ID layout changed")
        needle = struct.pack("<II", 0xb297, 0xf4b771)
        for match in re.finditer(re.escape(needle), data):
            pos = match.start()
            size = struct.unpack_from("<I", data, pos + row["allocParamSize"])[0]
            required, multi, any_parent = (data[pos + row[field]] for field in
                                          ["bParamRequired", "bMultiInstance", "bAnyParent"])
            free_priority = struct.unpack_from("<I", data, pos + row["freePriority"])[0]
            flags = struct.unpack_from("<I", data, pos + row["flags"])[0]
            parent = self.pointer(section, pos + row["pParentList"])
            parents = []
            parent_data = self.bytes(parent["section_index"])
            for i in range(16):
                value = struct.unpack_from("<I", parent_data, parent["offset"] + i * 4)[0]
                if value == 0:
                    break
                parents.append(hex(value))
            else:
                raise ValueError("unterminated parent class list")
            result["class_b297"].append({
                "section": ".data", "offset": hex(pos), "external_class": "0xb297",
                "internal_class": "0xf4b771", "alloc_parameter_size": size,
                "parameters_required": bool(required), "multi_instance": bool(multi),
                "any_parent": bool(any_parent), "parent_class_ids": parents,
                "free_priority": free_priority, "flags": hex(flags),
                "class_info": self.class_info(self.pointer(section, pos + row["pClassInfo"])),
            })
        section = self.index(".rodata")
        data = self.bytes(section)
        row = self.layout["NVOC_EXPORTED_METHOD_DEF"]
        if row["paramSize"] != row["methodId"] + 4:
            raise ValueError("reviewed adjacent method ID/size layout changed")
        for command, size in [(0x20801110, 8), (0x20801111, 40)]:
            needle = struct.pack("<II", command, size)
            for match in re.finditer(re.escape(needle), data):
                pos = match.start()
                base = pos - row["methodId"]
                function_offset = base + row["pFunc"]
                # Cross-check two relocations and the public Subdevice class ID.
                if (section, function_offset) in self.relocations:
                    function = self.pointer(section, function_offset)
                elif struct.unpack_from("<Q", data, function_offset)[0] == 0:
                    # Open RM metadata can route a halified call with no direct pFunc.
                    function = None
                else:
                    raise ValueError("nonzero function pointer lacks relocation")
                owner = self.class_info(self.pointer(section, base + row["pClassInfo"]))
                if owner["class_id"] != "0x4b01b3" or (function and function["section"] != ".text"):
                    raise ValueError("candidate is not a Subdevice control metadata row")
                flags = struct.unpack_from("<I", data, base + row["flags"])[0]
                access = struct.unpack_from("<I", data, base + row["accessRight"])[0]
                result["controls"].append({"command": hex(command), "parameter_size": size,
                    "section": ".rodata", "method_id_offset": hex(pos),
                    "flags": hex(flags), "access_right": access,
                    "function": function, "class_info": owner})
        return result


def inspect_installer(path, git_root):
    with path.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    version = next((v for v, h in INSTALLERS.items() if h == digest), None)
    if version is None:
        raise ValueError("installer not in reviewed SHA256 allowlist")
    metadata = compiled_metadata(git_root, version)
    output = {"driver_file_version": version, "installer_sha256": digest,
              "checksum_url": f"https://download.nvidia.com/XFree86/Linux-x86_64/{version}/"
                              f"NVIDIA-Linux-x86_64-{version}.run.sha256sum",
              "execution": "no installer/object execution; public layout probe only",
              "compiled_metadata": metadata, "objects": []}
    with path.open("rb") as stream:
        header = [stream.readline() for _ in range(12)]
        skip = int(next(line.split(b"=", 1)[1] for line in header if line.startswith(b"skip=")))
        stream.seek(0)
        for _ in range(skip - 1):
            stream.readline()
        # Synchronize the OS offset after Python buffered reads, before inheritance.
        os.lseek(stream.fileno(), stream.tell(), os.SEEK_SET)
        with subprocess.Popen(["zstd", "-d", "-q", "--stdout"], stdin=stream,
                              stdout=subprocess.PIPE) as decoder:
            with tarfile.open(fileobj=decoder.stdout, mode="r|") as archive:
                for member in archive:
                    if member.name not in ("kernel/nvidia/nv-kernel.o_binary",
                                           "kernel-open/nvidia/nv-kernel.o_binary"):
                        continue
                    if not member.isfile() or member.size > 200_000_000:
                        raise ValueError("unexpected archive member")
                    output["objects"].append({"archive_member": member.name,
                        **Elf(archive.extractfile(member).read(), metadata["layouts"]).inspect()})
            if decoder.wait() != 0:
                raise ValueError("zstd decompression failed")
    if len(output["objects"]) != 2:
        raise ValueError("expected proprietary and open kernel objects")
    return output


def inspect_names(root):
    # Discover identifiers from compiler output, then compile their values.
    # No raw-C numeric definitions are regex-parsed.
    with tempfile.TemporaryDirectory(prefix="kf-runlist-names-") as work:
        work = Path(work)
        probe = work / "headers.c"
        probe.write_text("".join('#include "' + path.name + '"\n' for path in
                                 sorted((root / "src/nvidia/generated").glob("*.h"))))
        includes = [root / p for p in INCLUDES] + sorted(
            {p.parent for p in (root / "src").rglob("*.h")})
        defines = ["-DNV_PRINTF_STRINGS_ALLOWED=0", "-DNVRM", "-DPORT_IS_KERNEL_BUILD=1",
                   "-DPORT_IS_CHECKED_BUILD=0", "-DPORT_MODULE_debug=1"]
        output = subprocess.check_output(["cc", "-E", "-dM", *defines,
            *["-I" + str(p) for p in includes], str(probe)], text=True)
        declarations = sorted(line for line in output.splitlines()
                              if line.startswith("#define __nvoc_class_id_"))
        names = [line.split()[1] for line in declarations]
        (work / "values.c").write_text('#include <stdio.h>\n' + "\n".join(declarations)
            + '\nint main(void) {\n' + "".join(
                'printf("' + name.removeprefix("__nvoc_class_id_") + ' %u\\n", '
                '(unsigned)' + name + ');\n' for name in names) + '}\n')
        subprocess.run(["cc", str(work / "values.c"), "-o", str(work / "values")], check=True)
        ids = {name: int(number) for name, number in (line.split() for line in
            subprocess.check_output([str(work / "values")], text=True).splitlines())}
    exceptions = [name for name, value in ids.items()
                  if int(hashlib.md5(name.encode()).hexdigest()[:6], 16) != value]
    if not ids or exceptions:
        raise ValueError("public NVOC MD5-prefix relation did not hold")
    return {"public_class_count": len(ids), "exceptions": exceptions,
            "value_origin": "compiler-preprocessed generated headers; numeric values compiled",
            "public_class_pairs_sha256": hashlib.sha256(
                json.dumps(ids, sort_keys=True).encode()).hexdigest(),
            "candidate_name": "RunlistApi",
            "candidate_md5": hashlib.md5(b"RunlistApi").hexdigest(),
            "qualification": "Name inference from 24-bit hash plus matching semantics; "
                             "not a published external-class definition or uniqueness proof."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--installer", type=Path, action="append", required=True)
    parser.add_argument("--ogkm-root", type=Path, required=True)
    parser.add_argument("--ogkm-git", type=Path, required=True,
                        help="public OGKM git repository containing pinned 535 and 610 commits")
    args = parser.parse_args()
    print(json.dumps({"schema": 2, "nvoc_name_relation": inspect_names(args.ogkm_root),
                      "installers": [inspect_installer(p, args.ogkm_git)
                                     for p in args.installer]}, indent=2))


if __name__ == "__main__":
    main()
