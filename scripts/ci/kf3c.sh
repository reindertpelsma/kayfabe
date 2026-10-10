#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# G7 tier 1: kf3.c compiles in CI (docs/design/V3_SEC_PERIMETER.md §8.1; OWNER_RULINGS §R rule d;
# audit S1-13). kf3.c turns Rust-supplied host pointers into QEMU memory regions, so it is inside
# the perimeter and is held to -Werror against a pristine, signature-checked QEMU 10.2.4.
#
#   bash scripts/ci/kf3c.sh
#
# Steps: the tarball (sha256 pinned in qemu-10.2.4.tar.xz.sha256, signer recorded there); the
# kf3 overlay applied to the pristine tree by build_kf3.sh's OWN lines (the `cp` of the overlay
# sources and its two one-line hunks, parsed, never edited; an empty archive stands in for
# libkf_qemu.a, which nothing here links); configure with build_kf3.sh's CONF_FLAGS plus
# --disable-download; generate the headers kf3.c's object needs; compile EVERY C file meson
# compiles for kf3 (from compile_commands.json, with kf3's own flags, `c_args` included) to an
# object with -Werror -Wextra; clang -fsyntax-only -Werror and clang --analyze -analyzer-werror;
# the link closure (the kf3_* symbols kf3's objects leave undefined equal kf-qemu's
# #[unsafe(no_mangle)] names); the depfile (K5). Known positives K0-K3 and K5 run every time: a
# case that cannot fail is not a check.
#
# ⊘ Until 2026-10-04 this compiled kf3.c with edu.c's command line (review): a `c_args` in kf3's
# meson.build, or a second C file in its `files(…)`, would ship through build_kf3.sh and never be
# compiled or analyzed here. Now kf3's own entries are the ones compiled, and `perimeter.py
# manifest` (K6) pins meson.build's hash and the overlay's file set.
#
# Environment: KF3C_WORK (default $RUNNER_TEMP/kf3c), KF3C_TARBALL (a local tarball to use),
# KF3C_BUILD (reuse an already configured build dir: local runs only).
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../.." && pwd)
ver=10.2.4
work="${KF3C_WORK:-${RUNNER_TEMP:-/tmp}/kf3c}"
kf3="$root/qemu/hw/misc/kf3"
fail=0
bad() { echo "★ $*"; fail=1; }
step() { printf '\n=== %s\n' "$*"; }
mkdir -p "$work"

# --- the tarball, pinned
want=$(grep -v '^#' "$here/qemu-$ver.tar.xz.sha256" | awk 'NF{print $1; exit}')
[ -n "$want" ] || { echo "★ no sha256 in qemu-$ver.tar.xz.sha256"; exit 2; }
if [ -z "${KF3C_BUILD:-}" ]; then
  tarball="${KF3C_TARBALL:-$work/qemu-$ver.tar.xz}"
  if [ ! -s "$tarball" ]; then
    step "download qemu-$ver.tar.xz"
    curl -fsSL -o "$tarball" "https://download.qemu.org/qemu-$ver.tar.xz"
  fi
  got=$(sha256sum "$tarball" | cut -d' ' -f1)
  [ "$got" = "$want" ] || { echo "★ qemu-$ver.tar.xz sha256 $got != pinned $want"; exit 2; }
  echo "qemu-$ver.tar.xz sha256 $got (pinned)"
  step "extract (no roms/)"
  rm -rf "$work/qemu-$ver" "$work/build"
  tar -xJf "$tarball" -C "$work" --exclude="qemu-$ver/roms"

  # --- build_kf3.sh's own configure line, parsed (that script is a bench tool; it is not edited)
  flags=$(python3 - "$root/scripts/bench/build_kf3.sh" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
m = re.search(r'^CONF_FLAGS="((?:[^"\\]|\\\n|\\.)*)"', text, re.M)
print(" ".join(m.group(1).replace("\\\n", " ").split()) if m else "")
PY
)
  [ -n "$flags" ] || { echo "★ could not extract CONF_FLAGS from scripts/bench/build_kf3.sh"; exit 2; }
  case "$flags" in *--enable-pixman*) ;; *) echo "★ CONF_FLAGS lacks --enable-pixman (kf3.c needs it)"; exit 2 ;; esac
  # --- build_kf3.sh's overlay, applied by its own lines: the `cp` of every overlay source and
  # the two hunks. Exactly these three lines must be found, or the bench script changed shape.
  overlay=$(python3 - "$root/scripts/bench/build_kf3.sh" <<'PY'
import re, sys
lines = open(sys.argv[1]).read().splitlines()
cp = [l for l in lines if re.match(r'^cp "\$REPO"/qemu/hw/misc/kf3/\S+ .*"\$QEMU/hw/misc/kf3/"$', l)]
hunks = [l for l in lines if re.match(r'^grep -q .*\|\| printf .*>> "\$QEMU/hw/misc/(meson\.build|Kconfig)"$', l)]
if len(cp) != 1 or len(hunks) != 2:
    sys.exit(f"overlay lines: cp={len(cp)} hunks={len(hunks)} (want 1 and 2)")
print("\n".join(cp + hunks))
PY
) || { echo "★ could not extract build_kf3.sh's overlay lines: $overlay"; exit 2; }
  step "overlay (build_kf3.sh's own lines)"
  echo "$overlay"
  mkdir -p "$work/qemu-$ver/hw/misc/kf3"
  # shellcheck disable=SC2034  # REPO and QEMU are read by the eval'd lines
  (set -e; REPO="$root"; QEMU="$work/qemu-$ver"; eval "$overlay")
  # the archive is only named by meson's link_args; nothing here links, so an empty one suffices
  ar rcs "$work/qemu-$ver/hw/misc/kf3/libkf_qemu.a"
  step "configure: $flags --disable-download"
  mkdir -p "$work/build"
  # shellcheck disable=SC2086  # a flag list by design
  (cd "$work/build" && "../qemu-$ver/configure" $flags --disable-download >"$work/configure.log" 2>&1) \
    || { tail -40 "$work/configure.log"; echo "★ configure failed"; exit 2; }
  build="$work/build"
else
  build="$KF3C_BUILD"
fi

# kf3's OWN compile entries: every C file meson compiles from the overlay directory.
mapfile -t entries < <(python3 - "$build/compile_commands.json" <<'PY'
import json, sys
es = [e for e in json.load(open(sys.argv[1])) if "/hw/misc/kf3/" in e["file"].replace("\\", "/")]
for e in es:
    print(e["file"].rsplit("/hw/misc/kf3/", 1)[1] + "\t" + e["output"])
PY
)
[ "${#entries[@]}" -ge 1 ] || { echo "★ meson compiles nothing from hw/misc/kf3 (is CONFIG_KF3 selected?)"; exit 2; }
printf 'kf3 compile entry: %s\n' "${entries[@]}"
case " ${entries[*]} " in *"kf3.c"*) ;; *) echo "★ no compile entry for kf3.c"; exit 2 ;; esac

step "generate the headers kf3's objects depend on"
hdrs=()
for e in "${entries[@]}"; do
  mapfile -t -O "${#hdrs[@]}" hdrs < <(cd "$build" && ninja -t query "${e#*$'\t'}" | awk '/^ *\|\| /{print $2}')
done
[ "${#hdrs[@]}" -gt 100 ] || { echo "★ only ${#hdrs[@]} generated headers found for kf3's objects"; exit 2; }
(cd "$build" && ninja "${hdrs[@]}" >/dev/null)
echo "headers: ${#hdrs[@]}"

# A file's own compile command, minus its source/output/depfile arguments
cmd_for() {
  python3 - "$build/compile_commands.json" "$1" <<'PY'
import json, shlex, sys
cmds = [e for e in json.load(open(sys.argv[1])) if e["file"].replace("\\", "/").endswith("/hw/misc/kf3/" + sys.argv[2])]
if len(cmds) != 1:
    sys.exit(f"{sys.argv[2]}: {len(cmds)} compile commands")
argv = shlex.split(cmds[0]["command"])
out, skip = [], 0
for i, a in enumerate(argv):
    if skip:
        skip -= 1
        continue
    if a in ("-MQ", "-MF", "-o", "-c"):
        skip = 1
        continue
    if a == "-MD" or a == cmds[0]["file"]:
        continue
    out.append(a)
print("\n".join(out))
PY
}
# clang knows neither gcc's `=N` forms nor -fzero-init-padding-bits; -Wno-unknown-warning-option
# covers gcc-only warning names. Prints the filtered flags, one per line.
to_clang() {
  for f in "$@"; do
    case "$f" in
      -Wimplicit-fallthrough=*) echo -Wimplicit-fallthrough ;;
      -Wshadow=*|-fzero-init-padding-bits=*|-fdiagnostics-color=*) ;;
      *) printf '%s\n' "$f" ;;
    esac
  done
}
base_txt=$(cmd_for kf3.c) || { echo "★ no compile command for kf3.c: $base_txt"; exit 2; }
mapfile -t base <<<"$base_txt"
cc_bin="${base[0]}"
flags_c=("${base[@]:1}")
T="$work/out"
rm -rf "$T"; mkdir -p "$T"
STRICT=(-Werror -Wextra -Wno-unused-parameter -Wno-sign-compare)

# Every OTHER C file meson compiles for kf3 (a second `kf3_*.c` in `files(…)`), from the CHECKOUT,
# with its own flags, -Werror -Wextra and the analyzer. (K6 already fails a new C file that
# perimeter.toml [c].files does not list.)
others=()
for e in "${entries[@]}"; do f="${e%%$'\t'*}"; [ "$f" = kf3.c ] || others+=("$f"); done
for f in "${others[@]}"; do
  step "gcc + clang analyzer: $f (kf3's own flags)"
  ob_txt=$(cmd_for "$f") || { bad "no compile command for $f"; continue; }
  mapfile -t ob <<<"$ob_txt"
  mapfile -t ocl < <(to_clang "${ob[@]:1}")
  (cd "$build" && "${ob[0]}" "${ob[@]:1}" "${STRICT[@]}" -c "$kf3/$f" -o "$T/${f%.c}.o") || bad "$f does not compile cleanly"
  (cd "$build" && clang "${ocl[@]}" -Wno-unknown-warning-option --analyze -Xanalyzer -analyzer-werror \
     -o "$T/${f%.c}.plist" "$kf3/$f") || bad "the clang static analyzer reports on $f"
done

step "gcc: kf3.c to an object, -Werror -Wextra ($("$cc_bin" --version | head -1))"
if ! (cd "$build" && "$cc_bin" "${flags_c[@]}" "${STRICT[@]}" -MD -MF "$T/kf3.d" -c "$kf3/kf3.c" -o "$T/kf3.o"); then
  bad "kf3.c does not compile cleanly"
fi

mapfile -t clang_flags < <(to_clang "${flags_c[@]}")
step "clang -fsyntax-only -Werror ($(clang --version | head -1))"
(cd "$build" && clang "${clang_flags[@]}" -Werror -Wno-unknown-warning-option -fsyntax-only "$kf3/kf3.c") \
  || bad "clang rejects kf3.c"
step "clang --analyze -analyzer-werror"
(cd "$build" && clang "${clang_flags[@]}" -Wno-unknown-warning-option --analyze -Xanalyzer -analyzer-werror \
   -o "$T/kf3.plist" "$kf3/kf3.c") || bad "the clang static analyzer reports on kf3.c"

step "link closure: kf3.o's undefined kf3_* symbols == kf-qemu's no_mangle exports"
rust_names=$(python3 - "$root" <<'PY'
import sys
sys.path.insert(0, sys.argv[1] + "/scripts/ci")
import rslex
from pathlib import Path
f = "crates/kf-qemu/src/ffi_unsafe.rs"
s = rslex.Structure(rslex.tokenize_file(Path(sys.argv[1]) / f, f), f)
names = []
for it in s.items:
    if it.kind == "fn" and any(s.attr_is(a, "unsafe", "(", "no_mangle", ")") for a in it.attrs):
        names.append(it.name)
print("\n".join(sorted(names)))
PY
)
c_names=$(for o in "$T"/*.o; do nm -u "$o" 2>/dev/null; done | awk '{print $NF}' | grep '^kf3_' | sort -u || true)
closure() {  # $1 = the Rust name list to compare against
  if [ "$1" != "$c_names" ]; then
    diff <(echo "$1") <(echo "$c_names") || true
    return 1
  fi
}
if closure "$rust_names"; then
  echo "closure: $(echo "$rust_names" | wc -l) = $(echo "$c_names" | wc -l)"
else
  bad "kf3.c's undefined kf3_* symbols differ from kf-qemu's no_mangle exports"
fi

step "K5: the in-tree headers were the ones compiled"
deps=$(tr ' \\' '\n\n' < "$T/kf3.d" | grep -E 'kf3[^/]*\.h$' | xargs -r -n1 realpath | sort -u || true)
want_deps=$(printf '%s\n' "$(realpath "$kf3/kf3.h")" "$(realpath "$kf3/kf3_gop.h")" | sort)
k5() { [ "$1" = "$want_deps" ]; }
k5 "$deps" || { echo "got: $deps"; bad "K5: kf3.d names other kf3 headers than the checkout's"; }

step "known positives (each must FAIL, or the check above proves nothing)"
k=$(mktemp -d "$work/k1.XXXX")
cp "$kf3/kf3.c" "$kf3/kf3.h" "$kf3/kf3_gop.h" "$k/"
# K0: -Werror is really on: one planted warning (an unused variable) fails the strict build.
cp "$k/kf3.c" "$k/k0.c"
printf '\nint kf3_k0_probe(void);\nint kf3_k0_probe(void) { int kf3_k0_unused = 0; return 0; }\n' >> "$k/k0.c"
if (cd "$build" && "$cc_bin" "${flags_c[@]}" "${STRICT[@]}" -c "$k/k0.c" -o "$k/k0.o" 2>"$k/err0"); then
  bad "K0: a planted warning compiled: -Werror is not in force"
else
  grep -q "kf3_k0_unused" "$k/err0" || bad "K0: failed, but not on the planted warning"
  echo "K0 ok: a planted warning failed the build"
fi
printf '\nint kf3_k1_probe(void);\nint kf3_k1_probe(void) { return kf3_k1_undeclared; }\n' >> "$k/kf3.c"
if (cd "$build" && "$cc_bin" "${flags_c[@]}" "${STRICT[@]}" -c "$k/kf3.c" -o "$k/kf3.o" 2>"$k/err"); then
  bad "K1: an undeclared identifier compiled"
else
  grep -q "kf3_k1_undeclared" "$k/err" || bad "K1: failed, but not on the planted identifier"
  echo "K1 ok: the planted undeclared identifier failed the build"
fi
(cd "$build" && "$cc_bin" "${flags_c[@]}" -E "$kf3/kf3.c" -o "$k/kf3.i") || bad "K2: cc -E failed"
if grep -q 'kf3_ov_apply' "$k/kf3.i"; then echo "K2 ok: the preprocessed unit is kf3.c's"; else bad "K2: kf3_ov_apply absent from cc -E"; fi
dropped=$(echo "$rust_names" | sed 1d)
if closure "$dropped" >/dev/null; then bad "K3: dropping a Rust export did not fail the closure"; else echo "K3 ok: one dropped export fails the closure"; fi
if k5 "$(printf '%s\n' "$deps" /elsewhere/kf3.h | sort)"; then bad "K5: a foreign kf3.h went unnoticed"; else echo "K5 ok: a foreign kf3.h fails the depfile check"; fi
rm -rf "$k"

echo
echo "KF3C fail=$fail"
exit "$fail"
