#!/usr/bin/env python3
"""
diff_traces_semantic.py — find the first SEMANTIC divergence between two
nvioctl_trace logs (host bare-metal vs guest in nvkvm).

Normalizes:
- pid/tid (per-run different)
- fd numbers (per-process different)
- nvidia RM client handles (pattern: XX YY d0 c1, low 16 bits randomized)
- userspace pointers (8-byte little-endian values whose high bytes match
  common user-VA patterns: 7f / 65 / 5f / 55 / 3a / 27)
- kernel VAs (rare, but masked when looks like the same pattern)

Reports the FIRST block whose POST or INNER POST differs after
normalization. That block's content is the upstream divergence; any
subsequent divergences are downstream consequences.

Usage:
  diff_traces_semantic.py HOST.log GUEST.log
"""
import re
import sys

VA_TAIL_BYTES = (0x7f, 0x65, 0x5f, 0x55, 0x3a, 0x27)


def parse_blocks(path):
    text = open(path).read()
    return text.split("\n\n")


def parse_block(text):
    cmd = re.search(r"CMD=(0x[0-9a-f]+)", text)
    cmd = cmd.group(1) if cmd else None
    inner = re.search(r"inner=(0x[0-9a-f]+)", text)
    inner = inner.group(1) if inner else None
    path = re.search(r"PATH=(\S+)", text)
    path = path.group(1) if path else None

    def get_bytes(label):
        m = re.search(label + r"\s*:\s*(?:\[\d+ bytes @ [^\]]+\]:\s*)?"
                              r"([0-9a-f ]+)", text)
        if not m:
            return None
        return [int(x, 16) for x in m.group(1).split() if len(x) == 2]

    return dict(
        cmd=cmd, inner=inner, path=path,
        pre=get_bytes("PRE"), post=get_bytes("POST"),
        ipre=get_bytes("INNER PRE"), ipost=get_bytes("INNER POST"),
    )


def normalize(bs):
    """Mask values that vary per-run but are semantically equivalent."""
    if bs is None:
        return None
    out = bs[:]
    n = len(out)

    # Mask 8-byte LE pointers — sliding window
    off = 0
    while off + 8 <= n:
        b5, b6, b7 = out[off + 5], out[off + 6], out[off + 7]
        # User VA: bytes 5..7 form one of "00 00 7f" / "00 00 65" / etc
        if b6 == 0 and b7 == 0 and b5 in VA_TAIL_BYTES:
            for k in range(8):
                out[off + k] = 0xAA
            off += 8
            continue
        # Kernel-internal pointer often has form 'XX XX YY ff ff ff ff ff'
        if (out[off + 3] == 0xff and out[off + 4] == 0xff and
                out[off + 5] == 0xff and out[off + 6] == 0xff and
                out[off + 7] == 0xff):
            for k in range(8):
                out[off + k] = 0xAA
            off += 8
            continue
        off += 1

    # Mask 4-byte nvidia client handles (pattern: XX YY d0 c1)
    off = 0
    while off + 4 <= n:
        if out[off + 2] == 0xd0 and out[off + 3] == 0xc1:
            out[off] = out[off + 1] = 0xAA
            off += 4
            continue
        off += 1

    return out


def diff(host_path, guest_path):
    h_blocks = parse_blocks(host_path)
    g_blocks = parse_blocks(guest_path)

    pairs = list(zip(h_blocks, g_blocks))
    found = 0

    for i, (hb, gb) in enumerate(pairs):
        ph, pg = parse_block(hb), parse_block(gb)
        if ph["cmd"] != pg["cmd"] or ph["inner"] != pg["inner"]:
            print(f"=== CMD/INNER ORDER DIVERGED at block {i} ===")
            print(f"  HOST : cmd={ph['cmd']} inner={ph['inner']} path={ph['path']}")
            print(f"  GUEST: cmd={pg['cmd']} inner={pg['inner']} path={pg['path']}")
            print()
            print("HOST block:")
            print(hb)
            print()
            print("GUEST block:")
            print(gb)
            return

        diffs = []
        for key in ("post", "ipost"):
            hn = normalize(ph[key])
            gn = normalize(pg[key])
            if hn is None and gn is None:
                continue
            if hn != gn:
                diffs.append(key)

        if diffs:
            found += 1
            if found > 5:
                continue
            print(f"=== SEMANTIC DIVERGENCE #{found} at block {i} ===")
            print(f"  cmd={ph['cmd']} inner={ph['inner']} path={ph['path']}")
            for key in diffs:
                hn = normalize(ph[key])
                gn = normalize(pg[key])
                print(f"  {key.upper()} host : "
                      + " ".join(f"{b:02x}" for b in hn))
                print(f"  {key.upper()} guest: "
                      + " ".join(f"{b:02x}" for b in gn))
                # Mark differing byte positions
                marks = []
                for x, y in zip(hn, gn):
                    marks.append("X " if x != y else ". ")
                print(f"  {key.upper()} diff : " + "".join(marks))
            print()

    extra_h = len(h_blocks) - len(pairs)
    extra_g = len(g_blocks) - len(pairs)
    print(f"Total semantic divergences: {found}")
    print(f"Host extra blocks: {extra_h}, guest extra blocks: {extra_g}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    diff(sys.argv[1], sys.argv[2])
