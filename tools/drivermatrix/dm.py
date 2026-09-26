#!/usr/bin/env python3
"""dm.py — MEASURE per-driver-version NVIDIA ABI facts by COMPILING ogkm at each tag.

★ Why this exists (`docs/design/V3_DRIVER_MATRIX.md` §3). kayfabe has two driver axes: the
GUEST driver talks to our fake GSP, the HOST driver answers our RM ioctls. Every struct layout,
RPC number and control id on either wire is defined per driver version, and the ogkm history
shows they move INSIDE a branch (GspSystemInfo gains fields at 580.95.05 and 580.105.08,
g_rpc-structures.h changes at 580.126.09, 580.159.04 and 580.173.02). A value read off one
tree and assumed for another is the defect this tool exists to end.

★ The parsers are the compiler, never regex over C (owner, 2026-09-21: *"use proper
parsers/compilers"*; `tools/derive_classes.sh` is the precedent):

- struct layouts: each type is instantiated in its own translation unit, compiled with `gcc -g`
  using the tag's OWN `src/nvidia/Makefile` flags, and the layout is read out of the DWARF the
  compiler emitted (every member, recursively: offset, size, type) — `pyelftools` walks it;
- enums: a variable of the enumerator's type is compiled, and DWARF lists every enumerator;
- `#define`s: names come from the preprocessor's own macro table (`gcc -E -dM`), values from a
  compiled `printf` of each name, so a value is what a compiler would use after conditionals.

★ Evidence discipline (from `nvkvm-pv/tools/abi_derive.sh`): one translation unit per entry, so
an entry that does not compile at a tag costs one MISSING row with its compiler error kept,
never the whole tag, and never a value borrowed from a neighbouring tag.

usage:
  dm.py fetch   --tag T --work DIR               sparse-clone the header trees of tag T
  dm.py probe   --src DIR --spec FILE --out DIR  measure one checkout against a spec
  dm.py sweep   --tags "T1 T2 .." --spec FILE --work DIR --out DIR [--keep]
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile

REPO = "https://github.com/NVIDIA/open-gpu-kernel-modules.git"

# The header trees every probe may need. Paths that do not exist at a tag are simply absent
# from its checkout (the vgpu headers moved from src/nvidia/kernel/inc to src/nvidia/inc/kernel
# at 565, for example); the include path list below carries both.
SPARSE = [
    "/src/nvidia/inc/",
    "/src/nvidia/generated/",
    "/src/nvidia/arch/nvalloc/common/inc/",
    "/src/nvidia/arch/nvalloc/unix/include/",
    "/src/nvidia/kernel/inc/",
    "/src/nvidia/interface/",
    "/src/nvidia/Makefile",
    "/src/common/sdk/nvidia/inc/",
    "/src/common/inc/",
    "/src/common/shared/",
    "/src/common/uproc/",
    "/src/common/nvswitch/common/inc/",
    "/src/common/nvswitch/interface/",
    "/src/common/nvswitch/kernel/inc/",
    "/src/common/nvlink/interface/",
    "/src/common/nvlink/inband/interface/",
    "/src/common/displayport/inc/",
    "/kernel-open/common/inc/",
    "/kernel-open/nvidia-uvm/",
    "/version.mk",
]


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


# ---------------------------------------------------------------------------------------------
# fetch
# ---------------------------------------------------------------------------------------------
def fetch(tag, work):
    dst = os.path.join(work, "src-" + tag)
    if os.path.isdir(os.path.join(dst, ".git")):
        return dst
    shutil.rmtree(dst, ignore_errors=True)
    r = run(["git", "clone", "-q", "--depth", "1", "--filter=blob:none", "--sparse",
             "--branch", tag, REPO, dst])
    if r.returncode != 0:
        raise SystemExit(f"fetch {tag}: clone failed: {r.stderr.strip()[:400]}")
    r = run(["git", "-C", dst, "sparse-checkout", "set", "--no-cone"] + SPARSE)
    if r.returncode != 0:
        raise SystemExit(f"fetch {tag}: sparse-checkout failed: {r.stderr.strip()[:400]}")
    return dst


# ---------------------------------------------------------------------------------------------
# the compile environment, read from the tag's own Makefile
# ---------------------------------------------------------------------------------------------
def env_flags(src, env):
    """gcc flags for one include environment.

    `sdk`  — the public SDK headers only (nvos.h, ctrl*/, class/, alloc/): the RM *ioctl* ABI a
             userspace client (kf-host, the raw client, libcuda) sees.
    `uvm`  — the SDK plus `kernel-open/nvidia-uvm` and `kernel-open/common/inc` (UVM ioctls).
    `rm`   — the RM's own build: every `CFLAGS += -I`/`-D` line at the top level of the tag's
             `src/nvidia/Makefile`, so an RM-internal header (the GSP RPC structures, the static
             info, the WPR metadata) compiles exactly as it does in the driver.
    """
    sdk = os.path.join(src, "src/common/sdk/nvidia/inc")
    common = os.path.join(src, "src/common/inc")
    base = ["-include", os.path.join(sdk, "cpuopsys.h"), "-I", sdk, "-I", common]
    if env == "sdk":
        return base
    if env == "uvm":
        return base + ["-I", os.path.join(src, "kernel-open/nvidia-uvm"),
                       "-I", os.path.join(src, "kernel-open/common/inc")]
    if env != "rm":
        raise SystemExit(f"unknown env {env}")
    mk = os.path.join(src, "src/nvidia/Makefile")
    flags = ["-include", os.path.join(sdk, "cpuopsys.h")]
    nvdir = os.path.join(src, "src/nvidia")
    commondir = os.path.join(src, "src/common")
    depth = 0
    with open(mk) as f:
        for line in f:
            s = line.strip()
            # Only UNCONDITIONAL top-level lines: an ifeq block is target-specific (arch,
            # compiler type, debug) and a header layout must not depend on which we guessed.
            if re.match(r"^(ifeq|ifneq|ifdef|ifndef)\b", s):
                depth += 1
                continue
            if s.startswith("endif"):
                depth = max(0, depth - 1)
                continue
            if depth or not s.startswith("CFLAGS +="):
                continue
            arg = s[len("CFLAGS +="):].strip()
            if arg.startswith("-I"):
                d = arg[2:].strip()
                if "$(" in d.replace("$(SRC_COMMON)", ""):
                    continue  # mbedtls/libspdm version-variable paths: not layout-relevant
                d = d.replace("$(SRC_COMMON)", commondir)
                if not os.path.isabs(d):
                    d = os.path.join(nvdir, d)
                flags += ["-I", os.path.normpath(d)]
            elif arg.startswith("-D"):
                if "MBEDTLS" in arg:
                    continue
                flags.append(arg.replace('"', ""))
    return flags


# ---------------------------------------------------------------------------------------------
# spec
# ---------------------------------------------------------------------------------------------
class Entry:
    def __init__(self, kind, name, env, headers, arg, lineno):
        self.kind, self.name, self.env, self.headers, self.arg, self.lineno = (
            kind, name, env, headers, arg, lineno)


def read_spec(path):
    """One entry per line: `kind  name  env  header[|header..]  arg`.

    kind = struct (arg = C type) | typedefs (arg = a Python regex over typedef NAMES, every
           match in the TU is measured — the names are discovered in the DWARF, never grepped)
           | enum (arg = one enumerator of the enum) | macros (arg = a Python regex over macro
           NAMES) | const (arg = a C expression).
    `header` is a comma list resolved against the tag's include path (headers move between
    releases: vgpu/ moved at 565); the include line written is the path given. A token `=X`
    emits `#define X`, `!X` emits `#undef X`, and `?h` includes `h` only where it exists — the RM's own include preambles
    (e.g. `rpc.c`'s `RPC_STRUCTURES`) are reproduced this way, never edited into a header.
    Alternatives separated by `|` are tried left to right — for RENAMED paths only.
    """
    out = []
    with open(path) as f:
        for n, line in enumerate(f, 1):
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            parts = line.split(None, 4)
            if len(parts) != 5:
                raise SystemExit(f"{path}:{n}: expected 5 columns, got {len(parts)}")
            kind, name, env, headers, arg = parts
            if kind not in ("struct", "typedefs", "enum", "macros", "const"):
                raise SystemExit(f"{path}:{n}: unknown kind {kind}")
            out.append(Entry(kind, name, env, headers.split("|"), arg, n))
    return out


# ---------------------------------------------------------------------------------------------
# DWARF
# ---------------------------------------------------------------------------------------------
def dwarf_layouts(obj, wanted):
    """{var_name: [(path, offset, size, typename)]} for each `wanted` global variable."""
    from elftools.elf.elffile import ELFFile

    res = {}
    with open(obj, "rb") as f:
        elf = ELFFile(f)
        dw = elf.get_dwarf_info()
        for cu in dw.iter_CUs():
            offs = {}
            for die in cu.iter_DIEs():
                offs[die.offset] = die
            for die in cu.iter_DIEs():
                if die.tag != "DW_TAG_variable":
                    continue
                nm = die.attributes.get("DW_AT_name")
                if not nm:
                    continue
                vname = nm.value.decode()
                if vname not in wanted:
                    continue
                t = die.get_DIE_from_attribute("DW_AT_type")
                rows = []
                flatten(t, "", 0, rows, 0)
                res[vname] = rows
    return res


def strip(t):
    while t is not None and t.tag in ("DW_TAG_typedef", "DW_TAG_const_type",
                                      "DW_TAG_volatile_type", "DW_TAG_restrict_type",
                                      "DW_TAG_atomic_type"):
        if "DW_AT_type" not in t.attributes:
            return None
        t = t.get_DIE_from_attribute("DW_AT_type")
    return t


def tname(t):
    """A short, stable name for a type — for review, never for layout decisions."""
    names = []
    while t is not None:
        nm = t.attributes.get("DW_AT_name")
        if t.tag == "DW_TAG_typedef" and nm:
            return nm.value.decode()
        if t.tag == "DW_TAG_pointer_type":
            return "ptr"
        if t.tag == "DW_TAG_array_type":
            return "array"
        if nm:
            return nm.value.decode()
        if t.tag in ("DW_TAG_structure_type",):
            return "struct"
        if t.tag in ("DW_TAG_union_type",):
            return "union"
        if t.tag in ("DW_TAG_enumeration_type",):
            return "enum"
        if "DW_AT_type" not in t.attributes:
            return t.tag
        t = t.get_DIE_from_attribute("DW_AT_type")
    return "?"


def type_size(t):
    s = strip(t)
    if s is None:
        return 0
    if "DW_AT_byte_size" in s.attributes:
        return s.attributes["DW_AT_byte_size"].value
    if s.tag == "DW_TAG_array_type":
        n = array_count(s)
        return n * type_size(s.get_DIE_from_attribute("DW_AT_type"))
    if s.tag == "DW_TAG_pointer_type":
        return 8
    return 0


def array_count(a):
    n = 1
    for sub in a.iter_children():
        if sub.tag != "DW_TAG_subrange_type":
            continue
        if "DW_AT_count" in sub.attributes:
            n *= sub.attributes["DW_AT_count"].value
        elif "DW_AT_upper_bound" in sub.attributes:
            n *= sub.attributes["DW_AT_upper_bound"].value + 1
        else:
            n *= 0  # flexible array member
    return n


def flatten(t, path, base, rows, depth):
    if depth > 24:
        raise SystemExit(f"type nesting deeper than 24 at {path}")
    s = strip(t)
    size = type_size(t)
    rows.append((path or ".", base, size, tname(t)))
    if s is None:
        return
    if s.tag in ("DW_TAG_structure_type", "DW_TAG_union_type"):
        for m in s.iter_children():
            if m.tag != "DW_TAG_member":
                continue
            nm = m.attributes.get("DW_AT_name")
            mname = nm.value.decode() if nm else "<anon>"
            loc = m.attributes.get("DW_AT_data_member_location")
            off = loc.value if loc is not None else 0
            if isinstance(off, list):
                raise SystemExit(f"unsupported member location expression at {path}.{mname}")
            if "DW_AT_bit_size" in m.attributes:
                bits = m.attributes["DW_AT_bit_size"].value
                boff = m.attributes.get("DW_AT_data_bit_offset")
                boff = boff.value if boff is not None else off * 8
                rows.append((f"{path}.{mname}" if path else mname, base + boff // 8,
                             -bits, f"bits@{boff % 8}"))
                continue
            sub = f"{path}.{mname}" if path else mname
            flatten(m.get_DIE_from_attribute("DW_AT_type"), sub, base + off, rows, depth + 1)
    elif s.tag == "DW_TAG_array_type":
        el = s.get_DIE_from_attribute("DW_AT_type")
        es = strip(el)
        # One element is enough: every element has the same layout. `[]` marks the stride
        # boundary; the element size is the next row's size.
        # ⊘ A FLEXIBLE array member is not recursed into: `rpc_message_header_v` ends in
        # `rpc_generic_union rpc_message_data[]`, the union of EVERY RPC body, which is measured
        # body by body instead.
        if array_count(s) and es is not None and es.tag in (
                "DW_TAG_structure_type", "DW_TAG_union_type", "DW_TAG_array_type"):
            flatten(el, f"{path}[]", base, rows, depth + 1)


# ---------------------------------------------------------------------------------------------
# probe
# ---------------------------------------------------------------------------------------------
def find_header(src, flags, suffix):
    dirs = [flags[i + 1] for i, a in enumerate(flags) if a == "-I"]
    for d in dirs:
        p = os.path.join(d, suffix)
        if os.path.isfile(p):
            return suffix
    return None


def compile_tu(src_text, flags, tmp, stem, link=False):
    c = os.path.join(tmp, stem + ".c")
    with open(c, "w") as f:
        f.write(src_text)
    out = os.path.join(tmp, stem + (".bin" if link else ".o"))
    cmd = ["gcc", "-w", "-g", "-O0", "-fno-eliminate-unused-debug-types"]
    cmd += flags + ([c, "-o", out] if link else ["-c", c, "-o", out])
    r = run(cmd)
    if r.returncode != 0:
        first = next((l for l in r.stderr.splitlines() if "error" in l), r.stderr[:200])
        return None, first.strip()
    return out, None


def includes_for(src, flags, e):
    for alt in e.headers:
        toks = alt.split(",")
        lines, ok = [], True
        for h in toks:
            if h.startswith("="):
                lines.append(f"#define {h[1:]}\n")
            elif h.startswith("!"):
                lines.append(f"#undef {h[1:]}\n")
            elif "*" in h:
                # a glob over the include path — every file the tag HAS under that pattern,
                # sorted (listing a directory is not parsing C). An empty match is not an error:
                # the entry then measures whatever the other tokens bring.
                import glob as _g
                dirs = [flags[i + 1] for i, a in enumerate(flags) if a == "-I"]
                hits = set()
                for d in dirs:
                    for fp in _g.glob(os.path.join(d, h)):
                        hits.add(os.path.relpath(fp, d))
                lines.extend(f'#include "{x}"\n' for x in sorted(hits))
            elif h.startswith("?"):
                # optional: included where the tag has it (a header that a branch adds or drops)
                if find_header(src, flags, h[1:]):
                    lines.append(f'#include "{h[1:]}"\n')
            elif find_header(src, flags, h):
                lines.append(f'#include "{h}"\n')
            else:
                ok = False
                break
        if ok:
            return "".join(lines), None
    return None, f"header(s) not found: {'|'.join(e.headers)}"


def probe(src, spec, outdir):
    os.makedirs(outdir, exist_ok=True)
    entries = read_spec(spec)
    flag_cache = {}
    lay = open(os.path.join(outdir, "layouts.tsv"), "w")
    val = open(os.path.join(outdir, "values.tsv"), "w")
    mis = open(os.path.join(outdir, "missing.tsv"), "w")
    with tempfile.TemporaryDirectory() as tmp:
        for i, e in enumerate(entries):
            if e.env not in flag_cache:
                flag_cache[e.env] = env_flags(src, e.env)
            flags = flag_cache[e.env]
            inc, err = includes_for(src, flags, e)
            if err:
                mis.write(f"{e.name}\t{err}\n")
                continue
            stem = f"e{i}"
            if e.kind == "struct":
                obj, err = compile_tu(f"{inc}{e.arg} __probe_v;\n", flags, tmp, stem)
                if err:
                    mis.write(f"{e.name}\t{err}\n")
                    continue
                rows = dwarf_layouts(obj, {"__probe_v"}).get("__probe_v")
                if not rows:
                    mis.write(f"{e.name}\tno DWARF for the probe variable\n")
                    continue
                for (p, off, size, tn) in rows:
                    lay.write(f"{e.name}\t{p}\t{off}\t{size}\t{tn}\n")
            elif e.kind == "typedefs":
                obj, err = compile_tu(f"{inc}int __probe_v;\n", flags, tmp, stem)
                if err:
                    mis.write(f"{e.name}\t{err}\n")
                    continue
                found = dwarf_typedefs(obj, re.compile(e.arg))
                if not found:
                    mis.write(f"{e.name}\tno typedef matches {e.arg}\n")
                for tdname, rows in found:
                    for (p, off, size, tn) in rows:
                        lay.write(f"{tdname}\t{p}\t{off}\t{size}\t{tn}\n")
            elif e.kind == "enum":
                # An enumerator's own type is `int` in C, so `typeof` cannot reach an anonymous
                # enum (`rpc_global_enums.h` is one). `-fno-eliminate-unused-debug-types` makes
                # gcc emit every enum the TU declares; the one containing `arg` is the answer.
                obj, err = compile_tu(f"{inc}int __probe_v = {e.arg};\n", flags, tmp, stem)
                if err:
                    mis.write(f"{e.name}\t{err}\n")
                    continue
                got = dwarf_enumerators(obj, e.arg)
                if not got:
                    mis.write(f"{e.name}\tno enumeration type contains {e.arg}\n")
                for (nm, v) in got:
                    val.write(f"{e.name}\t{nm}\t{v}\n")
            elif e.kind in ("macros", "const"):
                if e.kind == "macros":
                    names = macro_names(inc, flags, tmp, stem, e.arg)
                    if names is None:
                        mis.write(f"{e.name}\tpreprocessor failed\n")
                        continue
                    exprs = [(n, n) for n in names]
                else:
                    exprs = [(e.name, e.arg)]
                for (nm, v) in eval_exprs(inc, flags, tmp, stem, exprs):
                    if v is None:
                        mis.write(f"{e.name}\t{nm}: not a compile-time integer\n")
                    else:
                        val.write(f"{e.name}\t{nm}\t{v}\n")
    for f in (lay, val, mis):
        f.close()


def dwarf_typedefs(obj, rx):
    """[(typedef name, flattened layout)] for every typedef whose name fullmatches `rx` and
    whose target is a complete struct/union. First definition wins (one TU, one type)."""
    from elftools.elf.elffile import ELFFile

    out, seen = [], set()
    with open(obj, "rb") as f:
        elf = ELFFile(f)
        for cu in elf.get_dwarf_info().iter_CUs():
            for die in cu.iter_DIEs():
                if die.tag != "DW_TAG_typedef":
                    continue
                nm = die.attributes.get("DW_AT_name")
                if not nm:
                    continue
                name = nm.value.decode()
                if name in seen or not rx.fullmatch(name) or "DW_AT_type" not in die.attributes:
                    continue
                t = strip(die)
                if t is None or t.tag not in ("DW_TAG_structure_type", "DW_TAG_union_type"):
                    continue
                if "DW_AT_declaration" in t.attributes:
                    continue
                seen.add(name)
                rows = []
                flatten(die, "", 0, rows, 0)
                out.append((name, rows))
    return sorted(out)


def dwarf_enumerators(obj, member):
    """Every enumerator of the (first) enumeration type that declares `member`."""
    from elftools.elf.elffile import ELFFile

    with open(obj, "rb") as f:
        elf = ELFFile(f)
        for cu in elf.get_dwarf_info().iter_CUs():
            for die in cu.iter_DIEs():
                if die.tag != "DW_TAG_enumeration_type":
                    continue
                ens = [(en.attributes["DW_AT_name"].value.decode(),
                        en.attributes["DW_AT_const_value"].value)
                       for en in die.iter_children() if en.tag == "DW_TAG_enumerator"]
                if any(n == member for (n, _) in ens):
                    return ens
    return []


def macro_dump(text, flags, tmp, stem):
    c = os.path.join(tmp, stem + "_m.c")
    with open(c, "w") as f:
        f.write(text)
    r = run(["gcc", "-w", "-E", "-dM"] + flags + [c])
    if r.returncode != 0:
        return None
    names = set()
    for line in r.stdout.splitlines():
        # `-dM` prints `#define NAME BODY` or `#define NAME(args) BODY`; only object-like
        # macros can be values, and the NAME is the token right after `#define `.
        if not line.startswith("#define "):
            continue
        tok = line[8:].split(None, 1)[0]
        if "(" not in tok:
            names.add(tok)
    return names


def macro_names(inc, flags, tmp, stem, pattern):
    """Object-like macros the listed headers INTRODUCE (the macro table of the TU minus the
    macro table of an empty TU under the same flags), whose names fullmatch `pattern`."""
    got = macro_dump(inc, flags, tmp, stem)
    base = macro_dump("", flags, tmp, stem + "_base")
    if got is None or base is None:
        return None
    rx = re.compile(pattern)
    return sorted(n for n in got - base if rx.fullmatch(n))


# Only an INTEGER expression may become a value. A cast would happily turn a string literal or
# an address into a number (`(unsigned long long)"abc"` compiles), so each expression is gated
# by _Generic on its type and a non-integer prints NONINT instead of a plausible wrong number.
INT_GATE = """#define KF_IS_INT(x) _Generic((x), _Bool: 1, char: 1, signed char: 1, \
    unsigned char: 1, short: 1, unsigned short: 1, int: 1, unsigned: 1, long: 1, \
    unsigned long: 1, long long: 1, unsigned long long: 1, default: 0)
"""


def eval_exprs(inc, flags, tmp, stem, exprs):
    """Evaluate each expression as an unsigned 64-bit integer. One compile for the batch; a
    batch that does not compile is BISECTED, so one bad name costs O(log n) compiles and one
    MISSING row — never the batch, and never a value borrowed from elsewhere."""
    lflags = [f for f in flags if f != "-ffreestanding"]
    counter = [0]

    def prog(items):
        body = "".join(
            f'  if (KF_IS_INT({x})) printf("%s\\t%llu\\n", "{n}", '
            f'(unsigned long long)({x})); else printf("%s\\tNONINT\\n", "{n}");\n'
            for (n, x) in items)
        return f"#include <stdio.h>\n{inc}{INT_GATE}int main(void) {{\n{body}  return 0;\n}}\n"

    def go(items):
        counter[0] += 1
        exe, err = compile_tu(prog(items), lflags, tmp, f"{stem}_x{counter[0]}", link=True)
        if err:
            return None
        r = run([exe])
        if r.returncode != 0:
            return None
        out = []
        for line in r.stdout.splitlines():
            n, v = line.split("\t")
            out.append((n, None if v == "NONINT" else int(v)))
        return out

    def solve(items):
        if not items:
            return []
        got = go(items)
        if got is not None:
            return got
        if len(items) == 1:
            return [(items[0][0], None)]
        mid = len(items) // 2
        return solve(items[:mid]) + solve(items[mid:])

    return solve(exprs)


# ---------------------------------------------------------------------------------------------
# sweep
# ---------------------------------------------------------------------------------------------
def version_of(src):
    with open(os.path.join(src, "version.mk")) as f:
        for line in f:
            m = re.match(r"^NVIDIA_VERSION\s*=\s*(\S+)", line)
            if m:
                return m.group(1)
    return "?"


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("fetch")
    a.add_argument("--tag", required=True)
    a.add_argument("--work", required=True)
    b = sub.add_parser("probe")
    b.add_argument("--src", required=True)
    b.add_argument("--spec", required=True)
    b.add_argument("--out", required=True)
    c = sub.add_parser("sweep")
    c.add_argument("--tags", required=True)
    c.add_argument("--spec", required=True, action="append")
    c.add_argument("--work", required=True)
    c.add_argument("--out", required=True)
    c.add_argument("--keep", action="store_true")
    c.add_argument("--jobs", type=int, default=1)
    args = ap.parse_args()
    if args.cmd == "fetch":
        print(fetch(args.tag, args.work))
    elif args.cmd == "probe":
        probe(args.src, args.spec, args.out)
    else:
        tags = args.tags.split()
        if args.jobs <= 1:
            for tag in tags:
                sweep_one(tag, args)
        else:
            from concurrent.futures import ProcessPoolExecutor
            with ProcessPoolExecutor(max_workers=args.jobs) as ex:
                for _ in ex.map(sweep_one, tags, [args] * len(tags)):
                    pass


def sweep_one(tag, args):
    done = os.path.join(args.out, tag, ".done")
    if os.path.exists(done):
        print(f"CACHED {tag}", flush=True)
        return
    src = fetch(tag, args.work)
    v = version_of(src)
    if v != tag:
        raise SystemExit(f"{tag}: checkout says NVIDIA_VERSION={v}")
    for spec in args.spec:
        axis = os.path.splitext(os.path.basename(spec))[0]
        probe(src, spec, os.path.join(args.out, tag, axis))
    with open(done, "w") as f:
        f.write(tag + "\n")
    print(f"MEASURED {tag}", flush=True)
    if not args.keep:
        shutil.rmtree(src, ignore_errors=True)


if __name__ == "__main__":
    main()
