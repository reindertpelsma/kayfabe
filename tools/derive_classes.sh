#!/usr/bin/env bash
# ★★★ DERIVE the engine class ids by COMPILING ogkm's published headers — no parsing at all.
#
# `[owner, 2026-09-21]` *"don't use regex to parse C code (regex doesn't work on programming
# languages stable), use proper parsers/compilers etc."* — and *"use the proper parser per file,
# unless regex is the best option."*
#
# ⊘⊘ The first version of this script grepped `#define ... 0x...` out of the headers. That is
# wrong for the reason the owner gives: a `#define` can be conditional, can be redefined later in
# the translation unit, can expand through other macros, and can carry a suffix or a cast. A grep
# sees the first textual match and calls it the value. ⇒ **Here the C PREPROCESSOR is the parser**,
# and the value printed is the one a compiler would actually use.
#
# ## ★ Why this file uses a compiler and `swref` deliberately does not
#
# These headers are valid C: `#define ADA_COMPUTE_A 0xC9C0`.
# ⊘ The `swref` register headers are **not** — they carry `11:0` and
# `0x0003FFFF:0x00030000`, which are not C expressions, so a C parser yields **nothing** for them
# (`THE_V3_PLAN.md` P1 states this). For those a token-level parser is not a shortcut, it is the
# only thing that can read them, and `crates/kayfabe-doorbell/src/swref.rs` is that parser.
# ⇒ **The tool follows the grammar of the file, not a blanket rule.**
#
# Runs as any user. No driver, no device node, no blob, no root.
#
# usage: tools/derive_classes.sh [path-to-ogkm]
set -uo pipefail
OG=${1:-/workspace/nvidia-gpu-passthrough/research_clones/ogkm}
INC="$OG/src/common/sdk/nvidia/inc"
[ -d "$INC/class" ] || { echo "⊘ no class headers at $INC/class"; exit 2; }
command -v gcc >/dev/null || { echo "⊘ gcc is required — it IS the parser here"; exit 2; }

WANT="TURING_CHANNEL_GPFIFO_A TURING_COMPUTE_A TURING_DMA_COPY_A TURING_USERMODE_A
AMPERE_CHANNEL_GPFIFO_A AMPERE_COMPUTE_B AMPERE_DMA_COPY_B AMPERE_USERMODE_A
ADA_COMPUTE_A
HOPPER_CHANNEL_GPFIFO_A HOPPER_COMPUTE_A HOPPER_DMA_COPY_A HOPPER_USERMODE_A
BLACKWELL_CHANNEL_GPFIFO_A BLACKWELL_COMPUTE_A BLACKWELL_DMA_COPY_A BLACKWELL_USERMODE_A"

tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
{
  echo '#include <stdio.h>'
  # ⊘ Include every class header; the preprocessor resolves conditionals and redefinitions the
  # way the compiler would, which is the whole point of not grepping.
  for h in "$INC"/class/*.h; do echo "#include \"$h\""; done
  echo 'int main(void){'
  for n in $WANT; do
    # ⊘ `#ifdef` per symbol: a family whose header is absent must be REPORTED ABSENT, not
    # silently skipped or defaulted to zero.
    echo "#ifdef $n"
    echo "  printf(\"%-32s 0x%04X\\n\", \"$n\", (unsigned)($n));"
    echo '#else'
    echo "  printf(\"%-32s ABSENT\\n\", \"$n\");"
    echo '#endif'
  done
  echo '  return 0; }'
} > "$tmp/derive.c"

gcc -I"$INC" -I"$INC/class" -o "$tmp/derive" "$tmp/derive.c" 2>"$tmp/err" || {
  echo "⊘ compile failed — the headers did not build, which is itself the finding:"
  head -5 "$tmp/err"
  exit 3
}
"$tmp/derive"
