#!/usr/bin/env bash
# ★★★ DERIVE the per-FAMILY engine class SETS by COMPILING ogkm — no parsing of C at all.
#
# `[owner, 2026-09-21]` *"don't use regex to parse C code (regex doesn't work on programming
# languages stable), use proper parsers/compilers etc."* — and *"use the proper parser per file,
# unless regex is the best option."*
#
# ## ⊘⊘⊘ THE DEFECT THIS VERSION REPLACES (fable w824, HIGH 1 + MEDIUM 4)
#
# The first version of this script evaluated a HAND-PICKED list of sixteen symbols — one id per
# kind per family — and the Rust table claimed to be "transcribed by a generator". The ids were
# right; the SELECTION was ours, and it was per die-group: GA100 lists `AMPERE_COMPUTE_A 0xC6C0` /
# `AMPERE_DMA_COPY_A 0xC6B5`, GB202/GB20B list `BLACKWELL_COMPUTE_B 0xCEC0` /
# `BLACKWELL_DMA_COPY_B 0xCAB5` / `BLACKWELL_CHANNEL_GPFIFO_B 0xCA6F`, and the 3D class differs
# on EVERY family (`TURING_A 0xC597` … `BLACKWELL_B 0xCE97`). None were in the list, so an A100 or
# an RTX 50xx guest was denied its channel, compute and copy objects by default.
#
# ⇒ This version asks ogkm which classes EACH CHIP actually lists —
# `gpuGetEngClassDescriptorList_<CHIP>` in `src/nvidia/generated/g_gpu_class_list.c` — and unions
# them per family. Nothing is hand-picked: if a chip lists it, the family carries it.
#
# ## How, without parsing C
#
# 1. `g_gpu_class_list.c` is COMPILED, unmodified, against a shim include directory that supplies
#    the five headers it names. The shim's `CLASSDESCRIPTOR` carries the engine as a STRING
#    (`ENG_GR(0)` → `"GR0"`), so the compiler does the decoding of the initializer lists.
# 2. The chip names come from the compiled object's SYMBOL TABLE (`nm`), not from grepping the
#    source for function names.
# 3. Class ids are joined to symbol names through the PREPROCESSOR's macro table (`gcc -E -dM`):
#    a macro NAME that matches a kind pattern is evaluated by a compiled `printf`, so every id
#    is the value a compiler would use, after conditionals and redefinitions.
# 4. Chip → family is the ONE hand-maintained mapping (§50 level 6: per LARGE family, never per
#    die), and it follows ogkm's own `NV2080_CTRL_MC_ARCH_INFO_ARCHITECTURE_*` naming
#    (`ctrl2080mc.h:77-87`): TU→Turing, GA→Ampere, AD→Ada, GH→Hopper, GB/GR→Blackwell.
#    ⚠ GR100/GR102 carry their own ARCHITECTURE value (0x1C0, 610 only) but list ONLY Blackwell
#    engine classes; they are folded into Blackwell's SET because the set is what the allowlist
#    consults. Tegra parts (T23x/T264) are skipped: no discrete GPU, not a supported guest.
#
# Runs as any user. No driver, no device node, no blob, no root. `gcc` and `nm` ARE the parsers.
#
# usage: tools/derive_classes.sh [--rust] [path-to-ogkm]
#   default output: one `CLASS <Family> <kind> 0x<ID> <SYMBOL>` line per (family, id), plus one
#                   `FAMILY <Family> <CHIP>...` line per family — what the crate's test diffs.
#   --rust:         the body of `classgen::FAMILIES`, ready to paste.
set -uo pipefail
RUST=0
if [ "${1:-}" = "--rust" ]; then RUST=1; shift; fi
OG=${1:-/workspace/nvidia-gpu-passthrough/research_clones/ogkm}
INC="$OG/src/common/sdk/nvidia/inc"
GEN="$OG/src/nvidia/generated"
KINC="$OG/src/nvidia/inc/kernel"
LIST="$GEN/g_gpu_class_list.c"
[ -d "$INC/class" ] || { echo "⊘ no class headers at $INC/class"; exit 2; }
[ -f "$LIST" ] || { echo "⊘ no $LIST"; exit 2; }
command -v gcc >/dev/null || { echo "⊘ gcc is required — it IS the parser here"; exit 2; }
command -v nm >/dev/null || { echo "⊘ nm is required — the symbol table is how chips are found"; exit 2; }

tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/shim/core" "$tmp/shim/gpu"

# ---- 1. the shim: exactly the five headers g_gpu_class_list.c names ---------------------------
# ⊘ The real `core/core.h` and `gpu/gpu.h` pull in the whole RM object model. The class list
# needs four things from them: NvU32, NV_ARRAY_ELEMENTS, ct_assert, and the two typedefs.
cat > "$tmp/shim/core/core.h" <<'EOF'
#include <nvtypes.h>
#include <nvmisc.h>
#include <nvctassert.h>
EOF
cat > "$tmp/shim/gpu/eng_desc.h" <<'EOF'
/* The engine descriptor AS A STRING, so the compiler decodes `ENG_GR(0)` into "GR0" for us. */
typedef const char *ENGDESCRIPTOR;
#define ENG_STR_(x) #x
#define ENG_GR(x)                ("GR" ENG_STR_(x))
#define ENG_CE(x)                ("CE" ENG_STR_(x))
#define ENG_NVDEC(x)             ("NVDEC" ENG_STR_(x))
#define ENG_NVENC(x)             ("NVENC" ENG_STR_(x))
#define ENG_NVJPEG(x)            ("NVJPEG" ENG_STR_(x))
#define ENG_NVJPG                "NVJPG0"   /* object-like in ogkm, unlike ENG_NVJPEG(x) */
#define ENG_OFA(x)               ("OFA" ENG_STR_(x))
#define ENG_INVALID              "INVALID"
#define ENG_GPU                  "GPU"
#define ENG_KERNEL_FIFO          "KERNEL_FIFO"
#define ENG_KERNEL_DISPLAY       "KERNEL_DISPLAY"
#define ENG_KERNEL_MEMORY_SYSTEM "KERNEL_MEMORY_SYSTEM"
#define ENG_DMA                  "DMA"
#define ENG_SW                   "SW"
#define ENG_BUS                  "BUS"
#define ENG_SEC2                 "SEC2"
#define ENG_HDACODEC             "HDACODEC"
#define ENG_CONF_COMPUTE         "CONF_COMPUTE"
EOF
cat > "$tmp/shim/gpu/gpu.h" <<'EOF'
#include <core/core.h>
#include <gpu/eng_desc.h>
typedef struct OBJGPU OBJGPU;
typedef struct { NvU32 externalClassId; ENGDESCRIPTOR engDesc; } CLASSDESCRIPTOR;
EOF

CFLAGS="-I$tmp/shim -I$INC -I$GEN -I$OG/src/common/inc -w"

# ---- 2. compile the REAL class list, unmodified, and read the chips off its symbol table ------
gcc $CFLAGS -c -o "$tmp/list.o" "$LIST" 2>"$tmp/err" || {
  echo "⊘ g_gpu_class_list.c did not compile against the shim — which is itself the finding:"
  head -5 "$tmp/err"; exit 3
}
CHIPS=$(nm "$tmp/list.o" | awk '{print $NF}' | sed -n 's/^gpuGetEngClassDescriptorList_//p' | sort)
[ -n "$CHIPS" ] || { echo "⊘ no gpuGetEngClassDescriptorList_* symbols in the object"; exit 3; }

# ---- 3. the name table: macro names from the preprocessor, values from a compiled printf ------
{ for h in "$INC"/class/*.h; do echo "#include \"$h\""; done; } > "$tmp/all.h"
# ⊘ `-dM` prints the macro table the preprocessor ENDED with — the only names that survive
# conditionals. The pattern is applied to macro NAMES, not to C.
gcc $CFLAGS -E -dM "$tmp/all.h" | awk '{print $2}' \
  | grep -E '^[A-Z0-9]+_(CHANNEL_GPFIFO|COMPUTE|DMA_COPY|USERMODE)_[A-Z]$|^(TURING|AMPERE|ADA|HOPPER|BLACKWELL)_[A-Z]$|^NV[0-9A-F]{4}_VIDEO_(ENCODER|DECODER)$' \
  | sort -u > "$tmp/names"
{
  echo '#include <stdio.h>'
  cat "$tmp/all.h"
  echo 'int main(void){'
  while read -r n; do echo "  printf(\"0x%04X %s\\n\", (unsigned)($n), \"$n\");"; done < "$tmp/names"
  echo '  return 0; }'
} > "$tmp/names.c"
gcc $CFLAGS -o "$tmp/names.bin" "$tmp/names.c" 2>"$tmp/err" || { echo "⊘ name table failed:"; head -5 "$tmp/err"; exit 3; }
"$tmp/names.bin" | sort -u > "$tmp/id2name"   # "0xC5C0 TURING_COMPUTE_A"

# ---- 4. per-chip dump: call each list function and print (chip, id, engine) -------------------
{
  echo '#include <stdio.h>'
  echo '#include <gpu/gpu.h>'
  for c in $CHIPS; do echo "const CLASSDESCRIPTOR *gpuGetEngClassDescriptorList_$c(OBJGPU*, NvU32*);"; done
  echo 'int main(void){ NvU32 n; const CLASSDESCRIPTOR *d;'
  for c in $CHIPS; do
    echo "  d = gpuGetEngClassDescriptorList_$c(0, &n);"
    echo "  for (NvU32 i = 0; i < n; i++) printf(\"$c 0x%04X %s\\n\", d[i].externalClassId, d[i].engDesc);"
  done
  echo '  return 0; }'
} > "$tmp/dump.c"
gcc $CFLAGS -o "$tmp/dump.bin" "$tmp/dump.c" "$tmp/list.o" 2>"$tmp/err" || { echo "⊘ dump failed:"; head -5 "$tmp/err"; exit 3; }
"$tmp/dump.bin" | sort -u > "$tmp/chip_id_eng"   # "TU102 0xC5C0 GR0"

# ---- 5. chip → family: the one hand-maintained mapping (see header) ---------------------------
family_of() {
  case "$1" in
    TU*) echo Turing ;; GA*) echo Ampere ;; AD*) echo Ada ;; GH*) echo Hopper ;;
    GB*|GR*) echo Blackwell ;;
    *) echo "" ;;   # Tegra (T23x/T264): skipped
  esac
}
kind_of() {
  case "$1" in
    *_CHANNEL_GPFIFO_?) echo channel_gpfifo ;; *_COMPUTE_?) echo compute ;;
    *_DMA_COPY_?) echo dma_copy ;; *_USERMODE_?) echo usermode ;;
    *_VIDEO_ENCODER) echo video_encoder ;; *_VIDEO_DECODER) echo video_decoder ;;
    *) echo threed ;;   # the pattern in step 3 admits only <FAMILY>_<LETTER> here
  esac
}

# ---- 6. join and emit --------------------------------------------------------------------------
: > "$tmp/rows"
while read -r chip id eng; do
  fam=$(family_of "$chip"); [ -n "$fam" ] || continue
  name=$(awk -v i="$id" '$1==i{print $2; exit}' "$tmp/id2name"); [ -n "$name" ] || continue
  echo "$fam $(kind_of "$name") $id $name" >> "$tmp/rows"
done < "$tmp/chip_id_eng"
sort -u "$tmp/rows" > "$tmp/rows.u"

FAMS="Turing Ampere Ada Hopper Blackwell"
if [ "$RUST" = 0 ]; then
  for f in $FAMS; do
    chips=$(for c in $CHIPS; do [ "$(family_of "$c")" = "$f" ] && printf '%s ' "$c"; done)
    echo "FAMILY $f $chips"
  done
  awk '{print "CLASS", $1, $2, $3, $4}' "$tmp/rows.u"
else
  echo "// ★ GENERATED by tools/derive_classes.sh --rust from $(basename "$OG") — regenerate, never edit."
  for f in $FAMS; do
    chips=$(for c in $CHIPS; do [ "$(family_of "$c")" = "$f" ] && printf '"%s", ' "$c"; done)
    echo "    ClassSet {"
    echo "        family: Family::$f,"
    echo "        chips: &[${chips%, }],"
    for k in channel_gpfifo compute dma_copy usermode threed video_encoder video_decoder; do
      ids=$(awk -v f="$f" -v k="$k" '$1==f && $2==k {printf "%s /* %s */, ", $3, $4}' "$tmp/rows.u")
      echo "        $k: &[${ids%, }],"
    done
    echo "    },"
  done
fi
