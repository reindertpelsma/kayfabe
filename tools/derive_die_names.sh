#!/bin/sh
# Derive kf_chip::die::DIE_NAMES from an ogkm tree (tier 1 of the constants ruling) and diff it
# against crates/kf-chip/src/die.rs. Usage: tools/derive_die_names.sh <ogkm-root>
# Prints "OK" when every discrete-die IMPLEMENTATION define of ctrl2080mc.h is in die.rs with the
# same (architecture, implementation), else the differences. Integrated parts (GA10B, GB10B, GB20B,
# GB20C, GH100_SOC, T23x), the *00/*000 placeholders and the AD000/AD101 pair (both 0x1) are left
# out on purpose, as die.rs says.
set -eu
root=${1:?usage: derive_die_names.sh <ogkm-root>}
hdr=$(find "$root" -path '*ctrl2080/ctrl2080mc.h' | head -1)
[ -n "$hdr" ] || { echo "ctrl2080mc.h not found under $root" >&2; exit 2; }
here=$(cd "$(dirname "$0")/.." && pwd)
die_rs="$here/crates/kf-chip/src/die.rs"
python3 -I - "$hdr" "$die_rs" <<'EOF'
import re, sys
hdr, die_rs = sys.argv[1], sys.argv[2]
arch_of = {"TU": 0x160, "GA": 0x170, "GH": 0x180, "AD": 0x190}
skip = {"TU100", "GA000", "AD000", "AD100", "AD101", "GA10B", "GB10B", "GB20B", "GB20C", "GH100_SOC",
        "GP100", "GP000", "GB200", "T234", "T234D"}
want = {}
for m in re.finditer(r"#define\s+NV2080_CTRL_MC_ARCH_INFO_IMPLEMENTATION_(\w+)\s+\((0x[0-9A-Fa-f]+)\)", open(hdr).read()):
    name, imp = m.group(1), int(m.group(2), 16)
    if name in skip:
        continue
    if name.startswith("GB1"):
        a = 0x1A0
    elif name.startswith("GB2"):
        a = 0x1B0
    else:
        a = arch_of.get(name[:2])
    if a is None:
        continue
    want[name] = (a, imp)
consts = {"TU100": 0x160, "GA100": 0x170, "GH100": 0x180, "AD100": 0x190, "GB100": 0x1A0, "GB200": 0x1B0}
have = {}
for m in re.finditer(r"\(arch::(\w+),\s*(0x[0-9a-fA-F]+),\s*\"(\w+)\"\)", open(die_rs).read()):
    have[m.group(3)] = (consts[m.group(1)], int(m.group(2), 16))
diff = [f"{n}: ogkm {want.get(n)} die.rs {have.get(n)}" for n in sorted(set(want) | set(have)) if want.get(n) != have.get(n)]
print("OK" if not diff else "\n".join(diff))
sys.exit(1 if diff else 0)
EOF
