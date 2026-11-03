"""Diagnostic page builder for the PRECOMP bisect (KF3_DIAG_CAPS_PAGE_FILE input; never shipped, never committed).

The served (derived) page comes from `caps_oracle` output (its `derived` column; words equal to the native read
are not listed there). PRECOMP windows are 8 x 0x20 bytes at 0x780; word types 0..5 are CAPA..CAPF.
build(oracle, native, types): derived page, except that PRECOMP word types in `types` take the native value.
Usage: page_subset.py ORACLE.tsv NATIVE.txt OUT.txt TYPES   (TYPES like 0,1,2 or - for none)
"""
import sys

PRECOMP_BASE = 0x780
WINDOWS = 8
TYPES = 6
STRIDE = 0x20


def precomp_type(off: int):
    if off < PRECOMP_BASE or off >= PRECOMP_BASE + WINDOWS * STRIDE:
        return None
    k = ((off - PRECOMP_BASE) % STRIDE) // 4
    return k if k < TYPES else None


def build(oracle_text: str, native_text: str, types: set) -> dict:
    if any((not isinstance(t, int)) or t < 0 or t >= TYPES for t in types):
        raise ValueError("word types are 0..5")
    native = {}
    for l in native_text.splitlines():
        f = l.split()
        if f and f[0] == "WORD":
            native[int(f[1], 16)] = int(f[2], 16)
    derived = dict(native)  # a word absent from the oracle diff is equal in both pages
    for l in oracle_text.splitlines():
        if l.startswith("#") or l.startswith("offset") or not l.strip():
            continue
        f = l.split("\t")
        derived[int(f[0], 16)] = int(f[1], 16)
    out = {}
    for off, v in derived.items():
        k = precomp_type(off)
        if k is not None and k in types:
            v = native.get(off, 0)
        if v:
            out[off] = v
    # native words the derivation does not list and the native page has: already in `derived` via the copy
    for off in native:
        k = precomp_type(off)
        if k is not None and k in types and native[off]:
            out[off] = native[off]
    return out


def render(words: dict, label: str) -> str:
    lines = ["# DIAGNOSTIC page (%s); never shipped" % label]
    lines += ["WORD 0x%03x 0x%08x" % (o, words[o]) for o in sorted(words)]
    lines.append("# end words=%d" % len(words))
    return "\n".join(lines) + "\n"


def main(argv):
    types = set() if argv[4] == "-" else {int(x) for x in argv[4].split(",")}
    words = build(open(argv[1]).read(), open(argv[2]).read(), types)
    open(argv[3], "w").write(render(words, "types=" + argv[4]))
    print(len(words), "words")


if __name__ == "__main__":
    main(sys.argv)
