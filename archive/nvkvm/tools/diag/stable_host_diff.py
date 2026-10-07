#!/usr/bin/env python3
"""
stable_host_diff.py — paranoid host-vs-guest trace comparison.

Inputs:
  - N host trace logs (host_1.log, host_2.log, ...) all from runs that
    SUCCEEDED (cumemalloc PASS). The bytes that vary across these runs
    are per-run noise (process VAs, kernel-assigned handles, etc.) and
    must be IGNORED.
  - 1 guest trace log from a FAILING run.

Output:
  For each ioctl block, compare the guest bytes against the N host
  byte-sequences. A byte position is "stable in host" if it is the
  same value across all N host runs. Report blocks where any byte that
  is stable-in-host is DIFFERENT in guest. Those are the smoking guns.

  Reports both POST (outer params) and INNER POST (alloc params).

Usage:
  stable_host_diff.py guest.log host_1.log host_2.log host_3.log [...]

Output format:
  === SMOKING-GUN BLOCK <i>  cmd=<x> inner=<x> path=<p> ===
    POST host  (stable bytes shown; .. = noise across host runs)
    POST guest (highlighted where guest != stable host)
"""
import re
import sys


def parse(text):
    cmd = re.search(r"CMD=(0x[0-9a-f]+)", text)
    cmd = cmd.group(1) if cmd else None
    inner = re.search(r"inner=(0x[0-9a-f]+)", text)
    inner = inner.group(1) if inner else None
    path = re.search(r"PATH=(\S+)", text)
    path = path.group(1) if path else None

    def bytes_of(label):
        m = re.search(label + r"\s*:\s*(?:\[\d+ bytes @ [^\]]+\]:\s*)?"
                              r"([0-9a-f ]+)", text)
        if not m:
            return None
        return [int(x, 16) for x in m.group(1).split() if len(x) == 2]

    return dict(
        cmd=cmd, inner=inner, path=path,
        post=bytes_of("POST"), ipost=bytes_of("INNER POST"),
    )


def load_blocks(path):
    return [parse(b) for b in open(path).read().split("\n\n")]


def stability_mask(host_traces, block_idx, field):
    """
    For host_traces[*][block_idx][field], return a list of (stable_bool, value).
    stable_bool = True if all host runs have the same byte at this position.
    """
    cols = [t[block_idx].get(field) for t in host_traces]
    if any(c is None for c in cols):
        return None
    L = min(len(c) for c in cols)
    out = []
    for i in range(L):
        vals = {c[i] for c in cols}
        out.append((len(vals) == 1, cols[0][i]))
    return out


def fmt_diff(mask, guest_bytes):
    """Return a human-readable side-by-side: stable bytes shown literally,
    noise bytes as '..', guest bytes red-marked when stable-host != guest."""
    if mask is None or guest_bytes is None:
        return None, None, False
    host_str = []
    guest_str = []
    mark = []
    any_diff = False
    L = min(len(mask), len(guest_bytes))
    for i in range(L):
        stable, hval = mask[i]
        gval = guest_bytes[i]
        if stable:
            host_str.append(f"{hval:02x}")
            if hval != gval:
                guest_str.append(f"\033[1;31m{gval:02x}\033[0m")
                mark.append("X ")
                any_diff = True
            else:
                guest_str.append(f"{gval:02x}")
                mark.append(". ")
        else:
            host_str.append("..")
            guest_str.append("..")
            mark.append(". ")
    return " ".join(host_str), " ".join(guest_str), any_diff


def main(guest_path, host_paths):
    print(f"Loading guest: {guest_path}", file=sys.stderr)
    guest = load_blocks(guest_path)
    hosts = []
    for p in host_paths:
        print(f"Loading host:  {p}", file=sys.stderr)
        hosts.append(load_blocks(p))

    n_blocks = min(len(t) for t in hosts + [guest])
    print(f"Comparing {n_blocks} blocks (host runs: {len(hosts)})",
          file=sys.stderr)

    findings = 0
    for i in range(n_blocks):
        gb = guest[i]
        # Sanity: cmd should match across all
        ref_cmd = hosts[0][i]["cmd"]
        if gb["cmd"] != ref_cmd:
            print(f"=== ORDER DIVERGED at block {i} ===")
            print(f"  host  cmd={ref_cmd} inner={hosts[0][i]['inner']}")
            print(f"  guest cmd={gb['cmd']} inner={gb['inner']}")
            return

        for field in ("post", "ipost"):
            mask = stability_mask(hosts, i, field)
            if mask is None or gb.get(field) is None:
                continue
            host_str, guest_str, has_diff = fmt_diff(mask, gb[field])
            if not has_diff:
                continue
            findings += 1
            if findings > 10:
                continue
            print(f"=== SMOKING-GUN #{findings} block {i} "
                  f"cmd={gb['cmd']} inner={gb['inner']} path={gb['path']} ===")
            print(f"  {field.upper()} host (stable): {host_str}")
            print(f"  {field.upper()} guest        : {guest_str}")
            print()
    print(f"Total smoking-gun blocks: {findings}")


if __name__ == "__main__":
    if len(sys.argv) < 4:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    main(sys.argv[1], sys.argv[2:])
