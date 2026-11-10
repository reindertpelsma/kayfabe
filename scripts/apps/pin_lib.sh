# Fail-closed fetch helpers for the Linux apps hand path. Sourced by build_bundle.sh; defines functions only.
# Pins live in pins.sh. A helper returns non-zero (and prints one PIN_REFUSED line on stderr) for an empty,
# malformed or mismatching pin. There is no "unpinned, fetch anyway" branch anywhere in this file.
# Tests: scripts/ci/test_apps_pins.py (fake curl, local git remotes, fake dpkg/apt-get; no network).

pin_refuse() { echo "PIN_REFUSED: $*" >&2; return 1; }
pin_is_commit() { [[ ${1:-} =~ ^[0-9a-f]{40}$ ]]; }
pin_is_sha256() { [[ ${1:-} =~ ^[0-9a-f]{64}$ ]]; }

# pin_git_checkout DIR URL REF COMMIT
# DIR ends at exactly COMMIT, or this fails. REF (a tag) is only a hint for servers that refuse a fetch by
# commit; the check of HEAD against COMMIT is what pins. A DIR left by another revision is removed and refetched.
pin_git_checkout() {
  local dir=${1:?dir} url=${2:?url} ref=${3:-} want=${4:-} have
  pin_is_commit "$want" || { pin_refuse "git pin for $url is not a 40-hex commit: '$want'"; return 1; }
  if [ -d "$dir/.git" ]; then
    have=$(git -C "$dir" rev-parse HEAD 2>/dev/null || true)
    [ "$have" = "$want" ] && return 0
    rm -rf "$dir"
  fi
  rm -rf "$dir"
  git init -q "$dir" && git -C "$dir" remote add origin "$url" || return 1
  if ! git -C "$dir" fetch -q --depth 1 origin "$want" 2>/dev/null; then
    [ -n "$ref" ] || { rm -rf "$dir"; return 1; }
    git -C "$dir" fetch -q --depth 1 origin "refs/tags/$ref:refs/tags/$ref" || { rm -rf "$dir"; return 1; }
    git -C "$dir" checkout -q --detach "refs/tags/$ref" || { rm -rf "$dir"; return 1; }
  else
    git -C "$dir" checkout -q --detach FETCH_HEAD || { rm -rf "$dir"; return 1; }
  fi
  have=$(git -C "$dir" rev-parse HEAD)
  if [ "$have" != "$want" ]; then
    rm -rf "$dir"
    pin_refuse "$url: HEAD $have is not the pinned commit $want"
    return 1
  fi
  git -C "$dir" submodule update -q --init --recursive || { rm -rf "$dir"; return 1; }
}

# pin_verify_file PATH SHA256 [SIZE]: PATH is exactly the pinned bytes.
pin_verify_file() {
  local path=${1:?path} want=${2:-} size=${3:-} got
  pin_is_sha256 "$want" || { pin_refuse "sha256 pin for $path is empty or malformed: '$want'"; return 1; }
  [ -f "$path" ] || { pin_refuse "$path does not exist"; return 1; }
  if [ -n "$size" ] && [ "$(stat -c %s "$path")" != "$size" ]; then
    pin_refuse "$path: size $(stat -c %s "$path") is not the pinned $size"
    return 1
  fi
  got=$(sha256sum "$path" | cut -d' ' -f1)
  [ "$got" = "$want" ] || { pin_refuse "$path: sha256 $got is not the pinned $want"; return 1; }
}

# pin_fetch_file URL DEST SHA256 [SIZE]: DEST appears only when the download matches the pin.
pin_fetch_file() {
  local url=${1:?url} dest=${2:?dest} want=${3:-} size=${4:-}
  pin_is_sha256 "$want" || { pin_refuse "sha256 pin for $url is empty or malformed: '$want'"; return 1; }
  rm -f "$dest.part"
  curl -fsSL --retry 3 -o "$dest.part" "$url" || { rm -f "$dest.part"; return 1; }
  if ! pin_verify_file "$dest.part" "$want" "$size"; then
    rm -f "$dest.part"
    return 1
  fi
  mv -f "$dest.part" "$dest"
}

# pin_install_cuda_toolkit: the CUDA toolkit from NVIDIA's own repository, on this box, at run time.
# The keyring deb is verified against its per-release pin before dpkg sees it; a release without a pin is refused.
# PIN_OS_RELEASE_FILE exists for the tests.
pin_install_cuda_toolkit() {
  local rel var want dir have
  rel=$( . "${PIN_OS_RELEASE_FILE:-/etc/os-release}" && echo "ubuntu${VERSION_ID//./}" ) || return 1
  var=PIN_CUDA_KEYRING_SHA256_$rel
  want=${!var:-}
  pin_is_sha256 "$want" || { pin_refuse "no pinned CUDA keyring for $rel"; return 1; }
  dir=$(mktemp -d /tmp/cuda-keyring.XXXXXX) || return 1
  pin_fetch_file "$PIN_CUDA_KEYRING_URL_BASE/$rel/x86_64/$PIN_CUDA_KEYRING_DEB" "$dir/$PIN_CUDA_KEYRING_DEB" "$want" \
    || { rm -rf "$dir"; return 1; }
  dpkg -i "$dir/$PIN_CUDA_KEYRING_DEB" >/dev/null || { rm -rf "$dir"; return 1; }
  rm -rf "$dir"
  # ⚠ THE ONE DELIBERATE BYPASS OF THE CACHE-ONLY apt SHIM. Under the Vast payload, `apt-get` is a shell function
  # (vast_payload_hooks.sh, over vast_setup.py) that installs only from the reviewed setup cache, which holds no
  # NVIDIA package: the owner's ruling (2026-11-07) is that nothing from NVIDIA is stored in our images, R2 or
  # release assets. `command` skips the function, so these two calls reach the real /usr/bin/apt-get with the
  # box's own sources plus NVIDIA's repository (signed index, key pinned above). Every other apt-get in this
  # directory stays on the shim. Do not copy this pattern elsewhere.
  command apt-get -o DPkg::Lock::Timeout=300 update -qq || return 1
  command apt-get -o DPkg::Lock::Timeout=300 install -y -qq --no-install-recommends \
    "$PIN_CUDA_TOOLKIT_PACKAGE=$PIN_CUDA_TOOLKIT_VERSION" || return 1
  have=$(dpkg-query -W -f='${Version}' "$PIN_CUDA_TOOLKIT_PACKAGE" 2>/dev/null || true)
  [ "$have" = "$PIN_CUDA_TOOLKIT_VERSION" ] \
    || { pin_refuse "$PIN_CUDA_TOOLKIT_PACKAGE installed as '$have', pinned $PIN_CUDA_TOOLKIT_VERSION"; return 1; }
}
