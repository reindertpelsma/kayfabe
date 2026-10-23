#!/usr/bin/env bash
# Owner-invoked one-off dependency producer; does not rent or compile project code.
# Usage: bash scripts/ci/build_setup_cache.sh /absolute/new/output setup-cache-<tag>
set -euo pipefail
[[ $# == 2 && $1 == /* && ! -e $1 && $2 =~ ^setup-cache-[A-Za-z0-9._-]+$ ]] || exit 2
root=$(cd "$(dirname "$0")/../.." && pwd)
output=$1 tag=$2
mkdir -p "$output/context"
cp "$root/scripts/ci/setup-cache.Dockerfile" "$output/context/Dockerfile"
docker build --tag kf-setup-cache-builder "$output/context"
docker run --rm --cpus 2 --memory 2g -v "$root:/src:ro" -v "$output:/out" \
    kf-setup-cache-builder python3 /src/scripts/ci/setup_cache_build.py debs /src /out
(
    cd "$root"
    nice -n 19 env CARGO_HOME="$output/cargo-home" cargo vendor --locked --versioned-dirs \
        --sync firmware/kf-gop/Cargo.toml "$output/vendor" > "$output/vendor-config.txt"
)
python3 "$root/scripts/ci/setup_cache_build.py" lock "$root" "$output" "$tag"
printf '%s\n' "Review setup-cache.lock.json, then publish debs.tar.gz and vendor.tar.gz with gh release create."
