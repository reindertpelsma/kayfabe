#!/usr/bin/env python3
"""DERIVE the video-engine class method tables (NVDEC `*B0`, NVENC `*B7`, NVJPG `*D1`, OFA `*FA`)
from NVIDIA's PUBLIC class headers — by the preprocessor and the compiler, never by parsing C.

ogkm carries only the class ids of the video classes (`clc9b0.h` defines `NVC9B0_VIDEO_DECODER` and
nothing else); their method headers are published in NVIDIA/open-gpu-doc `classes/video/`. This
tool reads them as `derive_classes.sh` reads ogkm:

1. `gcc -E -dM` prints the header's macro TABLE (`#define NAME BODY`, one per line). Only the
   NAME and BODY of each line are taken; no C is parsed.
2. Every object-like macro of the class prefix (`NVC9B0_…`) is EVALUATED by a compiled program:
   a DRF field (a BODY of the NVIDIA `high:low` form) through the DRF ternary trick the NVIDIA
   headers are written for (`1 ? FIELD` = high bit, `0 ? FIELD` = low bit); every other macro as
   an integer constant expression.
3. Structure from NAMES only (NVIDIA's DRF naming convention): a METHOD is an integer macro that
   has at least one field `METHOD_<F>`; a field's named VALUES are the integer macros
   `FIELD_<V>`. `…_HIGH_FIELD` / `…_LOW_FIELD` (the `C5B7`-style bit metadata) are not values.

Usage: derive_video_methods.py <open-gpu-doc classes/video dir> <ogkm sdk inc dir> <rev> > out.rs
The <rev> (the open-gpu-doc commit) is written into the output header; a citation without a
revision is not a citation.
"""

import os
import re
import subprocess
import sys
import tempfile

# Only names (the files' and the macros'), never C text, are matched by these patterns.
HEADER_NAME = re.compile(r"^cl([0-9a-f]{2})(b0|b7|d1|fa)\.h$")
# Methods whose fields (and field values) the tier needs; every method's offset is emitted.
FIELDS_OF = (
    "SEMAPHORE_A",
    "SEMAPHORE_B",
    "SEMAPHORE_C",
    "SEMAPHORE_D",
    "SET_SEMAPHORE_PAYLOAD_LOWER",
    "SET_SEMAPHORE_PAYLOAD_UPPER",
)


def macro_table(header, inc):
    out = subprocess.run(
        ["gcc", "-E", "-dM", "-I", inc, header],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    table = {}
    for line in out.splitlines():
        parts = line.split(None, 2)
        if len(parts) < 3 or parts[0] != "#define" or "(" in parts[1]:
            continue  # function-like macros and empty defines are not constants
        table[parts[1]] = parts[2]
    return table


def evaluate(names, fields, header, inc):
    """Compile one program that prints every name's value (fields as high/low)."""
    with tempfile.TemporaryDirectory() as d:
        src = os.path.join(d, "eval.c")
        exe = os.path.join(d, "eval")
        with open(src, "w") as f:
            f.write("#include <stdio.h>\n#include \"%s\"\nint main(void){\n" % os.path.abspath(header))
            for n in names:
                if n in fields:
                    f.write('printf("F %%s %%d %%d\\n", "%s", (int)(1 ? %s), (int)(0 ? %s));\n' % (n, n, n))
                else:
                    f.write('printf("I %%s %%llu\\n", "%s", (unsigned long long)(%s));\n' % (n, n))
            f.write("return 0;}\n")
        subprocess.run(["gcc", "-w", "-I", inc, "-o", exe, src], check=True)
        out = subprocess.run([exe], check=True, capture_output=True, text=True).stdout
    ints, flds = {}, {}
    for line in out.splitlines():
        kind, name, *vals = line.split()
        if kind == "F":
            flds[name] = (int(vals[0]), int(vals[1]))
        else:
            ints[name] = int(vals[0])
    return ints, flds


def derive(header, inc):
    m = HEADER_NAME.match(os.path.basename(header))
    prefix = "NV%s%s_" % (m.group(1).upper(), m.group(2).upper())
    table = {k: v for k, v in macro_table(header, inc).items() if k.startswith(prefix)}
    # A DRF field's body is NVIDIA's `high:low` form; the compiler evaluates it below.
    fields = {k for k, v in table.items() if ":" in v}
    ints, flds = evaluate(sorted(table), fields, header, inc)
    meta = ("_HIGH_FIELD", "_LOW_FIELD")
    methods = {}
    for name, value in ints.items():
        if name.endswith(meta):
            continue
        if any(f.startswith(name + "_") for f in flds) and value % 4 == 0 and value < 0x4000:
            # a method unless it is the named value of a field (FIELD_<V>)
            if not any(name.startswith(f + "_") for f in flds):
                methods[name] = value
    rows = sorted((v, n[len(prefix):]) for n, v in methods.items())
    fields_of = {}
    for short in FIELDS_OF:
        full = prefix + short
        if full not in methods:
            continue
        mine = []
        for f, (hi, lo) in sorted(flds.items(), key=lambda kv: kv[1][1]):
            if not f.startswith(full + "_"):
                continue
            # the longest method prefix owns the field
            owner = max((mm for mm in methods if f.startswith(mm + "_")), key=len)
            if owner != full:
                continue
            values = sorted(
                (v, n[len(f) + 1 :])
                for n, v in ints.items()
                if n.startswith(f + "_") and not n.endswith(meta)
            )
            mine.append((f[len(full) + 1 :], hi, lo, values))
        fields_of[short] = mine
    cls = int(m.group(1) + m.group(2), 16)
    return cls, rows, fields_of


def header(rev, headers):
    """The comment block that opens the generated file (provenance first)."""
    return [
        "// @generated by tools/derive_video_methods.py — DO NOT EDIT BY HAND.",
        "//",
        "// Source: NVIDIA/open-gpu-doc @ %s, classes/video/{%s}" % (rev, ",".join(headers)),
        "// (ogkm carries only these classes' ids). Regenerate with:",
        "//   tools/derive_video_methods.py <open-gpu-doc>/classes/video \\",
        "//       <ogkm>/src/common/sdk/nvidia/inc <rev> > crates/kf-chan/src/video_generated.rs",
    ]


def main():
    src_dir, inc, rev = sys.argv[1:4]
    headers = sorted(h for h in os.listdir(src_dir) if HEADER_NAME.match(h))
    for line in header(rev, headers):
        print(line)
    print()
    print("//! The video-engine classes' method offsets (every method each public header names) and the")
    print("//! fields of the semaphore methods, evaluated by the compiler from NVIDIA's public headers.")
    print()
    print("use crate::vtables::{VideoClass, VideoField};")
    print()
    print("/// Every video class with a public method header, by class id.")
    print("pub static CLASSES: &[VideoClass] = &[")
    for h in headers:
        cls, rows, fields_of = derive(os.path.join(src_dir, h), inc)
        print("    VideoClass {")
        print("        class: %#06x," % cls)
        print("        methods: &[")
        for off, name in rows:
            print('            (%#x, "%s"),' % (off, name))
        print("        ],")
        print("        fields: &[")
        for short, mine in fields_of.items():
            for fname, hi, lo, values in mine:
                vals = ", ".join('("%s", %#x)' % (n, v) for v, n in values)
                print(
                    '            VideoField { method: "%s", name: "%s", hi: %d, lo: %d, values: &[%s] },'
                    % (short, fname, hi, lo, vals)
                )
        print("        ],")
        print("    },")
    print("];")


if __name__ == "__main__":
    main()
