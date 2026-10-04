#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# G1, the compiler location gate (docs/design/V3_SEC_PERIMETER.md §1.2; audit S1-01).
#
# Every compilation unit whose package lies in this checkout is compiled with the `unsafe_code`
# lint FORCED on (rustc reports every unsafe block, fn, impl, trait, extern block, unsafe
# attribute and global_asm!, after cfg and macro expansion, whatever the source or the command
# line says), and the gate fails unless every reported use lies in a `*_unsafe.rs` under
# `<P>/src/`, P being the unit's own class U package (scripts/ci/perimeter.toml), or wholly
# under the hash-bound exempt path.
#
#   bash scripts/ci/compiler_location.sh            # the gate
#   bash scripts/ci/compiler_location.sh --selftest # W1-W12: each must end the way it should
#
# What the compiler cannot see is the tokenizer's (`perimeter.py lex`): exported macro bodies,
# a same-package include!, code no pass compiles, doctests.
#
# Steps (the design's order): a clean environment pinned to the toolchain; the manifest gate;
# a fresh target directory and a protected copy of the gate; `debt.py frozen`; the passes
# (x86_64 with every feature, aarch64, each standalone package's runs); every unit reached;
# the location verdict; dep-info parity (L0); the size cross-check (SF4); an unchanged checkout;
# the protected copy unchanged; a summary line.
set -uo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

if [ "${1:-}" = --selftest ]; then
  exec python3 "$here/selftest_compiler_location.py"
fi

# --- 1. A clean environment: an allowlist, nothing inherited that could carry compiler flags.
if [ -z "${KF_LOC_CLEAN:-}" ]; then
  keep=(KF_LOC_CLEAN=1 "PATH=$PATH" "HOME=$HOME")
  for v in CARGO_HOME RUSTUP_HOME CARGO_TERM_COLOR RUNNER_TEMP KF_ROOT KF_TARGET_DIR KF_GATE_DIR \
           KF_ALLOW_REUSE_TARGET; do
    if [ -n "${!v:-}" ]; then keep+=("$v=${!v}"); fi
  done
  exec env -i "${keep[@]}" bash "$0" "$@"
fi

root=$(cd "${KF_ROOT:-$here/../..}" && pwd)
toml="$root/scripts/ci/perimeter.toml"
tmp="${RUNNER_TEMP:-/tmp}"
target="${KF_TARGET_DIR:-$tmp/kf-loc}"
gate="${KF_GATE_DIR:-$tmp/kf-gate}"
fail=0
step() { printf '\n=== %s\n' "$*"; }
bad() { echo "★ $*"; fail=1; }

pin=$(python3 "$here/perimeter.py" --root "$root" toolchain) || { echo "★ cannot read $toml"; exit 2; }
export RUSTUP_TOOLCHAIN="$pin"
step "1. toolchain $pin"
version=$(rustc -vV) || { echo "★ rustc -vV failed"; exit 2; }
echo "$version"
# (no pipe into `grep -q`: an early exit there is a SIGPIPE upstream, a failure manufactured by the check)
case "$version" in *"release: $pin"*) ;; *) echo "★ rustc is not $pin"; exit 2 ;; esac
pinned_rustc=$(rustup which rustc --toolchain "$pin") || { echo "★ rustup cannot find $pin"; exit 2; }

# --- 3 (before anything builds). A fresh target directory, and a protected copy of the gate.
step "3. fresh target $target; protected gate copy $gate"
if [ -e "$target" ] && [ -z "${KF_ALLOW_REUSE_TARGET:-}" ]; then
  echo "★ INFRASTRUCTURE: $target already exists. A warm target dir compiles nothing, so the wrapper"
  echo "  would see no unit and judge nothing. Remove it, or use a fresh path."
  exit 2
fi
rm -rf "$gate"
mkdir -p "$gate" "$target"
log="$gate/wraplog"
mkdir -p "$log"
for f in rustc_location_wrapper.py perimeter.py rslex.py dependencies.py; do
  cp "$here/$f" "$gate/$f"
done
cp "$toml" "$gate/perimeter.toml"
chmod 0555 "$gate"/*.py "$gate/perimeter.toml"
sums=$(cd "$gate" && sha256sum rustc_location_wrapper.py perimeter.py rslex.py dependencies.py perimeter.toml)
want=$( (cd "$here" && sha256sum rustc_location_wrapper.py perimeter.py rslex.py dependencies.py;
         cd "$root/scripts/ci" && sha256sum perimeter.toml) )
[ "$sums" = "$want" ] || { echo "★ the gate copy differs from the checkout"; exit 2; }
before=$(git -C "$root" status --porcelain)

# --- 2. The manifest gate first (M4: no committed flag injection).
step "2. manifests (M0-M5)"
python3 "$here/perimeter.py" --root "$root" manifest || bad "the manifest gate failed"

# --- 4. The exempt path is bound to its exact source.
frozen=""
if python3 - "$toml" <<'PY'
import sys, tomllib
ex = tomllib.load(open(sys.argv[1], "rb")).get("exempt", [])
bad = [e for e in ex if e.get("bound") != "debt.py frozen"]
sys.exit(2 if bad else (0 if ex else 1))
PY
then
  step "4. debt.py frozen (the exempt path's hash binding)"
  if python3 "$here/debt.py" frozen; then frozen=--frozen-ok; else bad "debt.py frozen failed"; fi
elif [ $? -eq 2 ]; then
  bad "an exempt row is not bound by \`debt.py frozen\`"
fi

# --- 5-7. The passes, every unit through the wrapper.
export RUSTC_WRAPPER="$gate/rustc_location_wrapper.py" RUSTC="$pinned_rustc" KF_PINNED_RUSTC="$pinned_rustc"
export KF_REPO_ROOT="$root" KF_PERIMETER_TOML="$gate/perimeter.toml" KF_WRAPLOG="$log"
while IFS=$'\t' read -r name manifest envs args; do
  # ★ One target directory PER PASS. Shared, a later pass compiles nothing a former one built
  # (build scripts are host units), and two standalone packages with the same name and version
  # hash identically because cargo hashes a root package by its workspace-relative path:
  # measured on CI 2026-10-04, kayfabe-abi/gen compiled ZERO units after kf-abi/gen.
  export CARGO_TARGET_DIR="$target/${name//[^A-Za-z0-9_-]/_}"
  step "5-7. pass $name: cargo check --manifest-path $manifest $args"
  envv=()
  if [ "$envs" != "-" ]; then IFS=, read -r -a envv <<<"$envs"; fi
  # ★ From the package's own directory: cargo reads `.cargo/config.toml` by the CURRENT directory, not
  # by --manifest-path, and a committed config is exactly what this gate must see (W5, W11).
  # shellcheck disable=SC2086
  if ! (cd "$(dirname "$root/$manifest")" && env "${envv[@]}" KF_PASS="$name" \
          cargo check --locked --keep-going --manifest-path "$root/$manifest" $args </dev/null); then
    bad "pass $name: cargo check failed"
  fi
done < <(python3 "$gate/perimeter.py" --root "$root" --config "$gate/perimeter.toml" passes)
unset RUSTC_WRAPPER RUSTC KF_PINNED_RUSTC CARGO_TARGET_DIR

# The gate copy is judged with BEFORE anything else reads it: a build script cannot have
# rewritten the verdict.
after_sums=$(cd "$gate" && sha256sum rustc_location_wrapper.py perimeter.py rslex.py dependencies.py perimeter.toml)
if [ "$after_sums" != "$sums" ]; then
  echo "★ the protected gate copy changed during the build:"; diff <(echo "$sums") <(echo "$after_sums")
  exit 1
fi
P=(python3 "$gate/perimeter.py" --root "$root" --config "$gate/perimeter.toml")

step "8. every target of every pass reached the wrapper"
"${P[@]}" reached --log "$log" || bad "unreached units"
step "9. location: every unsafe_code diagnostic inside its own class U crate's perimeter"
"${P[@]}" location --log "$log" $frozen | tee "$gate/location.txt" || bad "unsafe outside the perimeter"
step "10. dep-info parity (L0)"
"${P[@]}" depinfo --log "$log" || bad "dep-info parity"
if [ -f "$root/scripts/ci/perimeter/sizes.tsv" ]; then
  step "10b. size cross-check (SF4): the compiler's counts never exceed the tokenizer's"
  "${P[@]}" sizes --log "$log" || bad "size cross-check"
fi

step "11. the checkout is unchanged by the build"
after=$(git -C "$root" status --porcelain)
if [ "$after" != "$before" ]; then
  echo "★ a build step changed the checkout:"; diff <(echo "$before") <(echo "$after")
  fail=1
fi

summary=$(grep -o 'units=[0-9]* diagnostics=[0-9]* outside=[0-9]* exempt=[0-9]*' "$gate/location.txt" || true)
step "12. summary"
echo "COMPILER_LOCATION ${summary:-units=? diagnostics=? outside=? exempt=?} fail=$fail"
exit "$fail"
