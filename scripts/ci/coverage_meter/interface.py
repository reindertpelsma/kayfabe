# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Extract interface definitions from the pinned OGKM header inventories."""

import re
from collections import defaultdict
from dataclasses import dataclass

Key = tuple[str, int | None, int]
CONTROL = re.compile(r"NV([0-9A-F]+)_CTRL_CMD_[A-Z0-9_]+$")
DEFINE = re.compile(r"^[ \t]*#[ \t]*define[ \t]+([A-Za-z_]\w*)(\([^\n]*?\))?[ \t]+([^\n]+)", re.M)
INTEGER = re.compile(r"(0[xX][0-9a-fA-F]+|[0-9]+)[uUlL]*$")
BASE_OFFSET = re.compile(r"NV_IOCTL_BASE\s*\+\s*(\d+)$")


@dataclass(frozen=True, order=True)
class Definition:
    name: str
    oracle: str


def macros(source):
    source = re.sub(r"/\*.*?\*/|//[^\n]*", "", source.replace("\\\n", ""), flags=re.S)
    return [(name, args, body.strip()) for name, args, body in DEFINE.findall(source)]


def integer(body):
    match = INTEGER.fullmatch(body.strip().strip("() \t"))
    if match:
        value = match[1]
        return int(value, 16 if value.lower().startswith("0x") else 10)
    return None


def extract(root, oracle):
    sdk = root / "src/common/sdk/nvidia/inc"
    controls = sorted(sdk.glob("ctrl/**/*.h"))
    classes = sorted((sdk / "class").glob("cl*.h"))
    escapes = sorted(root.glob("src/nvidia/arch/nvalloc/unix/include/nv*.h"))
    escapes += sorted(root.glob("kernel-open/common/inc/nv-ioctl*.h"))
    ioctls = [root / p for p in (
        "kernel-open/nvidia-uvm/uvm_ioctl.h",
        "kernel-open/nvidia-uvm/uvm_linux_ioctl.h",
        "kernel-open/nvidia-uvm/uvm_test_ioctl.h",
    ) if (root / p).is_file()]
    if not controls or not classes or not escapes:
        raise ValueError(f"{oracle}: incomplete OGKM header tree")
    entries = defaultdict(set)
    unresolved = set()

    def add(kind, owner, value, name):
        entries[kind, owner, value].add(Definition(name, oracle))

    control_macros = [m for path in controls for m in macros(path.read_text(errors="replace"))]
    values = {name: body for name, args, body in control_macros if not args}
    for name, args, body in control_macros:
        match = CONTROL.fullmatch(name)
        if args or not match:
            continue
        visited = {name}
        while body.strip("() ") in values and body.strip("() ") not in visited:
            alias = body.strip("() ")
            visited.add(alias)
            body = values[alias]
        value = integer(body)
        # The CTRL_CMD prefix is also used for flags and enum fields. A
        # published RM command encodes the object class in the high word.
        owner = int(match[1], 16)
        if value is not None and value >> 16 == owner and (
            owner or value >= 0x100 or name.endswith("_NULL")
        ):
            add("control", owner, value, name)
        elif value is None and ":" not in body:
            unresolved.add(name)

    class_list = root / "src/nvidia/generated/g_allclasses.h"
    if class_list.is_file():
        for name, args, body in macros(class_list.read_text(errors="replace")):
            value = integer(body)
            if not args and value is not None and 0 <= value <= 0xffff:
                add("class", None, value, name)

    for path in classes:
        stem = re.fullmatch(r"cl([0-9a-fA-F]{4})", path.stem)
        if not stem:
            continue
        owner = int(stem[1], 16)
        definitions = macros(path.read_text(errors="replace"))
        names = [n for n, args, b in definitions if not args and integer(b) == owner
                 and not n.startswith(f"NV{stem[1].upper()}_")]
        for name in names or [path.stem]:
            add("class", None, owner, name)
        prefix = f"NV{stem[1].upper()}_"
        fields = {n.rsplit("_", 1)[0] for n, _, body in definitions if re.fullmatch(r"\d+:\d+", body)}
        # Scalar method definitions use zero-padded hex. Indexed definitions
        # publish a base plus a stride. Do not inventory enum values or masks.
        for name, args, body in definitions:
            if not name.startswith(prefix):
                continue
            match = re.match(r"^\(?\s*(0x[0-9a-fA-F]{8})\b", body)
            if not match:
                continue
            offset = int(match[1], 16)
            indexed = args and re.search(rf"\b{re.escape(args[1:-1])}\b", body)
            scalar = not args and integer(body) is not None and (
                name in fields or name == prefix + "SET_OBJECT"
                or (offset >= 0x100 and name.startswith((prefix + "SET_", prefix + "GET_"))
                    and not any(name.startswith(field + "_") for field in fields))
            )
            if 0 <= offset <= 0x7ffc and offset % 4 == 0 and (indexed or scalar):
                add("method", owner, offset, name)

    escape_macros = [m for path in escapes for m in macros(path.read_text(errors="replace"))]
    base = next((integer(body) for name, _, body in escape_macros
                 if name == "NV_IOCTL_BASE"), None)
    for name, args, body in escape_macros:
        if args or not name.startswith("NV_ESC_"):
            continue
        value = integer(body)
        offset = BASE_OFFSET.fullmatch(body.strip("() "))
        if value is None and offset and base is not None:
            value = base + int(offset[1])
        if value is None:
            unresolved.add(name)
        else:
            add("escape", None, value, name)

    for path in ioctls:
        for name, args, body in macros(path.read_text(errors="replace")):
            match = re.fullmatch(r"UVM_(TEST_)?IOCTL_BASE\((\d+)\)", body)
            if not args and match:
                add("ioctl", None, int(match[2]) + (200 if match[1] else 0), name)
    drm = root / "kernel-open/nvidia-drm/nvidia-drm-ioctl.h"
    if drm.is_file():
        definitions = macros(drm.read_text(errors="replace"))
        offsets = {name: integer(body) for name, args, body in definitions if not args}
        for name, args, body in definitions:
            match = re.search(r"DRM_COMMAND_BASE\s*\+\s*(DRM_NVIDIA_\w+)", body)
            if not args and name.startswith("DRM_IOCTL_NVIDIA_") and match:
                offset = offsets.get(match[1])
                if offset is not None:
                    add("drm_ioctl", None, 0x40 + offset, name)
    modeset = root / "kernel-open/nvidia-modeset/nvkms-ioctl.h"
    if modeset.is_file():
        for name, args, body in macros(modeset.read_text(errors="replace")):
            value = integer(body)
            if not args and name == "NVKMS_IOCTL_CMD" and value is not None:
                add("modeset_ioctl", None, value, "NVKMS_IOCTL_IOWR")
    return entries, unresolved
