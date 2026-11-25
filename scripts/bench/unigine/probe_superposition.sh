#!/bin/sh
# Superposition 1.1 Basic probe: download (official URL), hash, extract with --noexec as an
# unprivileged user, then show that superposition_cli exits silently when ~/.Superposition/license
# is absent (the CLI is a Pro-edition feature). Usage: probe_superposition.sh <scratch-dir>
# Never run postinstall.sh (it only writes .desktop files). Files stay out of git.
set -eu
D=${1:?scratch dir}
mkdir -p "$D/dl"
cd "$D/dl"
for e in run exe; do
  [ -s "Unigine_Superposition-1.1.$e" ] || curl -fsSL -o "Unigine_Superposition-1.1.$e" "https://assets.unigine.com/d/Unigine_Superposition-1.1.$e"
done
sha256sum Unigine_Superposition-1.1.* | tee SHA256.txt
sh Unigine_Superposition-1.1.run --list | head -40
[ -d "$D/app" ] || sh Unigine_Superposition-1.1.run --noexec --nox11 --target "$D/app"
cd "$D/app/bin"
if [ -f "$HOME/.Superposition/license" ]; then echo "license present"; else echo "no license file"; fi
DISPLAY=${DISPLAY:-:0} ./superposition_cli -xml "$D/app/bin/pro_xml_samples/single_run_low.xml"; echo "cli rc=$? (0 with no output and no report = license gate)"
