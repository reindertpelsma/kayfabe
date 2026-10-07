#!/usr/bin/env bash
# ★★★ DERIVE the per-CHIP hardware reference table — register offsets, bit fields, apertures,
# in-memory structure bits, class-method layouts and USERD/notifier struct offsets — by COMPILING
# ogkm's own headers. No parsing of C at all. The hardware-side counterpart of the ioctl ABI table.
#
# `[owner, 2026-09-21]` *"don't use regex to parse C code … use proper parsers/compilers"* — the rule
# `tools/derive_classes.sh` follows, and this script follows it the same way:
#
# 1. Which macros exist is asked of the PREPROCESSOR (`gcc -E -dM`, per header), and the spec's
#    prefixes (`tools/hwref_spec.txt`) are matched against macro NAMES — never against bodies.
# 2. Every body is evaluated by the COMPILER, with ogkm's own accessors from `nvmisc.h`:
#      pass 1  `(unsigned long long)(X)`                 a plain value (an offset, an enum value)
#      pass 2  `DRF_EXTENT(X)`, `DRF_BASE(X)`            a `hi:lo` bit range, an aperture, or a
#                                                         structure bit range (`(34*32+31):(34*32+0)`)
#      pass 3  `DRF_PICK_MW(X,1)`, `DRF_PICK_MW(X,0)`    a multi-word `MW(hi:lo)` field (fault buffer
#                                                         entries) — nvmisc.h's own `DRF_EXPAND_MW`
#    A name that fails a pass (the compiler says which line) moves to the next; one that fails all
#    three (an empty body, a string) is dropped and counted on stderr.
#    ⊘ This corrects `kayfabe-doorbell/src/swref.rs`'s premise that a `hi:lo` body leaves "no value
#    to evaluate": ogkm itself evaluates every such body through `DRF_BASE(drf) (0?drf)` /
#    `DRF_EXTENT(drf) (1?drf)` (`nvmisc.h:233-234`), and so does this script.
# 3. Function-like macros of one parameter (`NV_PGSP_QUEUE_HEAD(i)`) are evaluated at `(0)` and
#    `(1)`, which is the base and the stride. Two or more parameters: dropped and counted.
# 4. Struct member offsets (`struct` spec lines) are `offsetof`/`sizeof` — the compiler's layout.
#
# ⊘ This script only EXTRACTS: one row per (directory, macro). Which directory a GPU family's
# driver actually compiles against — and when two candidate directories disagree — is decided in
# ONE place, `kf_chip::hwref` (the family lineage + named pins), and its tests hold kayfabe's
# hand-written constants to the answer.
#
# Directories: every discrete-GPU chip directory under `src/common/inc/swref/published/<arch>/`
# (kepler … blackwell, incl. the pre-Turing ancestors newer families inherit definitions from), the
# root `nv_ref.h`, UVM's own copies under `kernel-open/nvidia-uvm/hwref/` (labelled `uvm/<arch>/<chip>`),
# and the SDK class headers (labelled `class`). Tegra, NVSwitch, display and bridge trees are skipped.
#
# Runs as any user. No driver, no device node, no root. `gcc` IS the parser.
#
# usage: tools/derive_hwref.sh [path-to-ogkm] > crates/kf-chip/data/hwref-<version>.tsv
#        tools/derive_hwref.sh --check FILE [path-to-ogkm]   exit 0 iff FILE equals a fresh derivation
set -uo pipefail
CHECK=""
if [ "${1:-}" = "--check" ]; then CHECK=${2:?--check needs a file}; shift 2; fi
OG=${1:-/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04}
HERE=$(cd "$(dirname "$0")" && pwd)
SPEC=${HWREF_SPEC:-$HERE/hwref_spec.txt}
PUB="$OG/src/common/inc/swref/published"
UVMREF="$OG/kernel-open/nvidia-uvm/hwref"
SDKINC="$OG/src/common/sdk/nvidia/inc"
CLS="$SDKINC/class"
[ -d "$PUB" ] || { echo "⊘ no published swref headers at $PUB" >&2; exit 2; }
[ -f "$SDKINC/nvmisc.h" ] || { echo "⊘ no $SDKINC/nvmisc.h" >&2; exit 2; }
[ -f "$SPEC" ] || { echo "⊘ no spec $SPEC" >&2; exit 2; }
command -v gcc >/dev/null || { echo "⊘ gcc is required — it IS the parser here" >&2; exit 2; }

tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
CFLAGS="-std=gnu11 -w -ftrack-macro-expansion=0 -I$SDKINC -I$OG/src/common/inc"

# ---- 0. the spec: our own format, read line by line ------------------------------------------
: > "$tmp/swref.pfx"; : > "$tmp/class.list"; : > "$tmp/struct.list"
section=""
while IFS= read -r line; do
  line=${line%%#*}; set -- $line; [ $# -gt 0 ] || continue
  case "$1" in
    swref) section=swref ;;
    class) section=class; echo "$2" >> "$tmp/class.list" ;;
    struct) section=struct; echo "$*" >> "$tmp/struct.list" ;;
    *) [ "$section" = swref ] && echo "$1" >> "$tmp/swref.pfx" ;;
  esac
done < "$SPEC"

# name filter: `PREFIX` matches a name starting with it; `NAME$` matches exactly. Names only.
awk_filter='
  NR==FNR { if ($1 ~ /\$$/) exact[substr($1,1,length($1)-1)]=1; else pfx[++np]=$1; next }
  { n=$1; if (n in exact) { print; next } for (i=1;i<=np;i++) if (index(n,pfx[i])==1) { print; next } }'

# ---- 1. per header: its MACRO TABLE, as the preprocessor reports it ----------------------------
# ⊘ A few published headers are not self-contained C (`blackwell/gb100/dev_nv_pcie_config_reg_
# addendum.h` declares an array over registers no published header defines). Only the MACROS are
# wanted, so each header is replaced by its own `gcc -E -dM` table — the preprocessor's canonical
# output, minus the compiler's predefined names (asked of an empty translation unit) — and the
# evaluation program includes those tables. The compiler still evaluates every body.
# "Predefined" = what the evaluation context itself brings: the compiler's own names plus
# `nvtypes.h`/`nvmisc.h` (which the class headers include and every evaluation program includes).
printf '#include "nvtypes.h"\n#include "nvmisc.h"\n' | gcc $CFLAGS -E -dM - \
  | awk '{ n=$2; sub(/\(.*/,"",n); print n }' | sort -u > "$tmp/predef"
mkdir -p "$tmp/mac"
macro_table() {  # $1 = header path, $2 = output file
  gcc $CFLAGS -E -dM "$1" 2>/dev/null \
    | awk 'NR==FNR { pre[$1]=1; next } { n=$2; sub(/\(.*/,"",n) } !(n in pre)' "$tmp/predef" - > "$2"
}
# prints "NAME<TAB>HEADER<TAB>ARITY" (ARITY: -1 object-like, else the parameter count) from a table
names_of() {  # $1 = macro-table file, $2 = the header's name
  awk -v h="$2" '
    $1=="#define" { n=$2; a=-1
      if ((p=index(n,"("))>0) { args=substr(n,p+1); n=substr(n,1,p-1); sub(/\).*/,"",args)
        a = (args=="") ? 0 : split(args, _, ",") }
      print n "\t" h "\t" a }' "$1"
}

# ---- 2. evaluate a list of (expr, header) in the context of an include list ------------------
# $1 = file with "EXPR<TAB>HEADER" lines, $2 = file with #include lines, $3 = label; stdout rows.
evaluate() {
  local list=$1 incs=$2 label=$3 pass file i
  cp "$list" "$tmp/todo"
  for pass in v r m; do
    [ -s "$tmp/todo" ] || break
    file="$tmp/eval_$pass.c"
    {
      echo '#include <stdio.h>'
      echo '#include "nvtypes.h"'
      echo '#include "nvmisc.h"'
      cat "$incs"
      echo '#define HWREF_PICK(d,v) DRF_PICK_MW(d,v)'
      echo '#define HWREF_N(x) ((unsigned long long)(x))'
      echo 'static void hwref_rng(const char*h,const char*n,const char*k,unsigned long long hi,unsigned long long lo){'
      echo '  printf("%s\t%s\t%s", h, n, k);'
      echo '  if (hi < 4096) printf("%llu:", hi); else printf("0x%llx:", hi);'
      echo '  if (lo < 4096) printf("%llu\n", lo); else printf("0x%llx\n", lo); }'
      # one function per line: the compiler's error line IS the row it rejects
      i=0
      while IFS=$'\t' read -r e h; do
        i=$((i+1))
        case $pass in
          v) echo "static void f$i(void){ printf(\"%s\\t%s\\t0x%llx\\n\", \"$h\", \"$e\", HWREF_N($e)); }" ;;
          r) echo "static void f$i(void){ hwref_rng(\"$h\", \"$e\", \"\", HWREF_N(DRF_EXTENT($e)), HWREF_N(DRF_BASE($e))); }" ;;
          m) echo "static void f$i(void){ hwref_rng(\"$h\", \"$e\", \"mw:\", HWREF_N(HWREF_PICK($e,1)), HWREF_N(HWREF_PICK($e,0))); }" ;;
        esac
      done < "$tmp/todo"
      echo "int main(void){"
      for ((j=1; j<=i; j++)); do echo "  f$j();"; done
      echo "  return 0; }"
    } > "$file"
    local first
    first=$(grep -n '^static void f1(void)' "$file" | cut -d: -f1)
    # which lines does the compiler reject?
    gcc $CFLAGS -fsyntax-only "$file" 2> "$tmp/err_$pass" || true
    if grep -q ': error:' "$tmp/err_$pass" && grep ': error:' "$tmp/err_$pass" | grep -vq "^$file:"; then
      echo "⊘ $label: an error outside the generated file (a header does not compile):" >&2
      grep ': error:' "$tmp/err_$pass" | grep -v "^$file:" | head -3 >&2; exit 3
    fi
    # ⚠ the "0" sentinel keeps the list non-empty: awk's `NR==FNR` idiom would otherwise read the
    # SECOND file as the first when this one is empty, and silently drop every row of the pass.
    { echo 0; grep "^$file:[0-9]*:[0-9]*: error:" "$tmp/err_$pass" | cut -d: -f2 | sort -un \
      | awk -v f="$first" '{ print $1 - f + 1 }'; } > "$tmp/bad"
    # keep the accepted rows for this pass; the rejected ones go on to the next
    awk 'NR==FNR { bad[$1]=1; next } !(FNR in bad)' "$tmp/bad" "$tmp/todo" > "$tmp/ok"
    awk 'NR==FNR { bad[$1]=1; next }  (FNR in bad)' "$tmp/bad" "$tmp/todo" > "$tmp/todo.next"
    if [ -s "$tmp/ok" ]; then
      # rebuild with the accepted rows only, compile, run
      { sed -n "1,$((first-1))p" "$file"
        awk 'NR==FNR { bad[$1]=1; next } !(FNR in bad) { print }' "$tmp/bad" <(grep '^static void f[0-9]*(void)' "$file")
        echo "int main(void){"
        awk 'NR==FNR { bad[$1]=1; next } !(FNR in bad) { print "  f" FNR "();" }' "$tmp/bad" "$tmp/todo"
        echo "  return 0; }"
      } > "$file.ok.c"
      gcc $CFLAGS -o "$tmp/eval.bin" "$file.ok.c" 2> "$tmp/errb" \
        || { echo "⊘ $label pass $pass: accepted rows do not build:" >&2; head -3 "$tmp/errb" >&2; exit 3; }
      "$tmp/eval.bin" | awk -F'\t' -v d="$label" '{ print d "\t" $0 }'
    fi
    mv "$tmp/todo.next" "$tmp/todo"
  done
  if [ -s "$tmp/todo" ]; then
    echo "$label: $(wc -l < "$tmp/todo") name(s) with no evaluable body (empty, string, or not a range):" \
      "$(cut -f1 "$tmp/todo" | head -6 | tr '\n' ' ')" >> "$tmp/dropped"
  fi
}

: > "$tmp/dropped"; : > "$tmp/rows"
# ---- 3. the swref + UVM hwref directories ------------------------------------------------------
dirs() {
  echo "published	$PUB/nv_ref.h"
  for arch in kepler maxwell pascal volta turing ampere ada hopper blackwell; do
    for d in "$PUB/$arch"/*/; do [ -d "$d" ] && echo "$arch/$(basename "$d")	$d"; done
  done
  for arch in maxwell pascal volta turing ampere ada hopper blackwell; do
    for d in "$UVMREF/$arch"/*/; do [ -d "$d" ] && echo "uvm/$arch/$(basename "$d")	$d"; done
  done
}
while IFS=$'\t' read -r label path; do
  if [ -f "$path" ]; then hdrs="$path"; else hdrs=$(ls "$path"*.h 2>/dev/null); fi
  [ -n "$hdrs" ] || continue
  : > "$tmp/names"; : > "$tmp/incs"; k=0
  for h in $hdrs; do
    k=$((k+1)); m="$tmp/mac/${label//\//_}_$k.h"; macro_table "$h" "$m"
    names_of "$m" "$(basename "$h")" >> "$tmp/names"; echo "#include \"$m\"" >> "$tmp/incs"
  done
  # first header to define a name owns it; filter by the spec's name prefixes
  awk -F'\t' '!seen[$1]++' "$tmp/names" | awk "$awk_filter" "$tmp/swref.pfx" - > "$tmp/sel"
  [ -s "$tmp/sel" ] || continue
  awk -F'\t' '$3==-1 { print $1 "\t" $2 } $3==1 { print $1 "(0)\t" $2; print $1 "(1)\t" $2 }
              $3>1 { print "skip" > "/dev/stderr" }' "$tmp/sel" 2>>"$tmp/multiarg" > "$tmp/list"
  evaluate "$tmp/list" "$tmp/incs" "$label" >> "$tmp/rows"
done < <(dirs)

# ---- 4. class headers (taken whole; the predefined names are already out of the tables) ---------
while read -r f; do
  [ -f "$CLS/$f" ] || { echo "⊘ spec names $f, which ogkm does not have" >&2; exit 2; }
  m="$tmp/mac/class_$f"; macro_table "$CLS/$f" "$m"; echo "#include \"$m\"" > "$tmp/incs"
  names_of "$m" "$f" | awk -F'\t' '$1 !~ /^_/ && !seen[$1]++' > "$tmp/sel"
  awk -F'\t' '$3==-1 { print $1 "\t" $2 } $3==1 { print $1 "(0)\t" $2; print $1 "(1)\t" $2 }' "$tmp/sel" > "$tmp/list"
  evaluate "$tmp/list" "$tmp/incs" "class" >> "$tmp/rows"
done < "$tmp/class.list"

# ---- 5. struct layouts ---------------------------------------------------------------------------
while read -r _ f type members; do
  if [ -f "$CLS/$f" ]; then hp="$CLS/$f"; else hp="$SDKINC/$f"; fi
  [ -f "$hp" ] || { echo "⊘ spec names $f, which ogkm does not have" >&2; exit 2; }
  for m in $members; do
    { echo '#include <stdio.h>'; echo '#include <stddef.h>'; echo '#include "nvtypes.h"'
      echo "#include \"$hp\""
      echo "int main(void){ printf(\"class\\t$f\\t$type.$m\\toff=0x%zx\\n\", offsetof($type, $m)); return 0; }"
    } > "$tmp/s.c"
    if gcc $CFLAGS -o "$tmp/s.bin" "$tmp/s.c" 2>/dev/null; then "$tmp/s.bin" >> "$tmp/rows"
    else printf 'class\t%s\t%s.%s\tabsent\n' "$f" "$type" "$m" >> "$tmp/rows"; fi
  done
  { echo '#include <stdio.h>'; echo '#include "nvtypes.h"'; echo "#include \"$hp\""
    echo "int main(void){ printf(\"class\\t$f\\tsizeof($type)\\tsize=0x%zx\\n\", sizeof($type)); return 0; }"
  } > "$tmp/s.c"
  gcc $CFLAGS -o "$tmp/s.bin" "$tmp/s.c" 2>/dev/null && "$tmp/s.bin" >> "$tmp/rows"
done < "$tmp/struct.list"

# ---- 6. emit --------------------------------------------------------------------------------------
ver=$(awk -F' = ' '$1=="NVIDIA_VERSION"{print $2}' "$OG/version.mk" 2>/dev/null)
rev=$(git -C "$OG" rev-parse --short HEAD 2>/dev/null || echo unknown)
{
  echo "# ★ GENERATED by tools/derive_hwref.sh from ogkm ${ver:-?} (${rev}) with tools/hwref_spec.txt — regenerate, never edit."
  echo "# columns: dir<TAB>header<TAB>name<TAB>value — value is 0x<hex> (a value), <hi>:<lo> (a bit range, aperture"
  echo "# or structure-bit range; decimal below 4096), mw:<hi>:<lo> (a multi-word field), off=0x<n> / size=0x<n> / absent"
  echo "# (a struct member). Family resolution (which dir a family compiles against) lives in kf_chip::hwref."
  LC_ALL=C sort -t$'\t' -k1,1 -k3,3 -u "$tmp/rows"
} > "$tmp/out"
if [ -s "$tmp/dropped" ] || [ -s "$tmp/multiarg" ]; then
  { cat "$tmp/dropped"; [ -s "$tmp/multiarg" ] && echo "$(wc -l < "$tmp/multiarg") multi-parameter macro(s) skipped"; } >&2
fi
if [ -n "$CHECK" ]; then
  if diff -q <(grep -v '^#' "$CHECK") <(grep -v '^#' "$tmp/out") >/dev/null; then
    echo "✔ $CHECK matches a fresh derivation from $OG"; exit 0
  fi
  echo "⊘ $CHECK differs from a fresh derivation from $OG:" >&2
  diff <(grep -v '^#' "$CHECK") <(grep -v '^#' "$tmp/out") | head -20 >&2; exit 1
fi
cat "$tmp/out"
