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
  if ! pin_retry git -C "$dir" fetch -q --depth 1 origin "$want" 2>/dev/null; then
    [ -n "$ref" ] || { rm -rf "$dir"; return 1; }
    pin_retry git -C "$dir" fetch -q --depth 1 origin "refs/tags/$ref:refs/tags/$ref" || { rm -rf "$dir"; return 1; }
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
  pin_retry git -C "$dir" submodule update -q --init --recursive || { rm -rf "$dir"; return 1; }
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
  # Up to PIN_RETRY_ATTEMPTS tries: a failed or truncated download is tried again; the digest is checked after EVERY
  # attempt, so a persistent mismatch is still refused (never installed), only later.
  pin_retry pin_fetch_once "$url" "$dest" "$want" "$size" && return 0
  pin_refuse "$url: no download matched the pin $want after ${PIN_RETRY_ATTEMPTS:-3} attempts"
  return 1
}

# One attempt of pin_fetch_file. Stalls and dead connects end the attempt (curl's own bounds, no total time here: the
# stage's provisioning cap is the total bound).
pin_fetch_once() {
  local url=$1 dest=$2 want=$3 size=$4
  rm -f "$dest.part"
  curl -fsSL --connect-timeout 20 --speed-limit 51200 --speed-time 60 -o "$dest.part" "$url" || { rm -f "$dest.part"; return 1; }
  if ! pin_verify_file "$dest.part" "$want" "$size"; then
    rm -f "$dest.part"
    return 1
  fi
  mv -f "$dest.part" "$dest"
}

# pin_retry CMD...: run CMD up to PIN_RETRY_ATTEMPTS times (default 3), sleeping the PIN_RETRY_BACKOFF seconds
# (space separated, default "3 10") between attempts. Returns the last status. No retry of anything that is not a fetch.
pin_retry() {
  # Local names carry a prefix so that a callee (which shares our dynamic scope) cannot collide with them. The attempts
  # value must be a number from 1 to 5 and every backoff a number of seconds up to 999: anything else fails closed to
  # the defaults (3 attempts, 3 s), never to a loop without end.
  local _pr_attempts=${PIN_RETRY_ATTEMPTS:-3} _pr_n=1 _pr_rc _pr_waits _pr_w
  [[ $_pr_attempts =~ ^[1-5]$ ]] || _pr_attempts=3
  read -r -a _pr_waits <<<"${PIN_RETRY_BACKOFF:-3 10}"
  [ "${#_pr_waits[@]}" -gt 0 ] || _pr_waits=(3)
  while true; do
    "$@"
    _pr_rc=$?
    [ "$_pr_rc" -eq 0 ] && return 0
    [ "$_pr_n" -ge "$_pr_attempts" ] && return "$_pr_rc"
    _pr_w=${_pr_waits[$((_pr_n - 1))]:-${_pr_waits[${#_pr_waits[@]} - 1]}}
    [[ $_pr_w =~ ^[0-9]{1,3}$ ]] || _pr_w=3
    sleep "$_pr_w"
    _pr_n=$((_pr_n + 1))
  done
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
  command apt-get -o DPkg::Lock::Timeout=300 -o Acquire::Retries=3 update -qq || return 1
  command apt-get -o DPkg::Lock::Timeout=300 -o Acquire::Retries=3 install -y -qq --no-install-recommends \
    "$PIN_CUDA_TOOLKIT_PACKAGE=$PIN_CUDA_TOOLKIT_VERSION" || return 1
  have=$(dpkg-query -W -f='${Version}' "$PIN_CUDA_TOOLKIT_PACKAGE" 2>/dev/null || true)
  [ "$have" = "$PIN_CUDA_TOOLKIT_VERSION" ] \
    || { pin_refuse "$PIN_CUDA_TOOLKIT_PACKAGE installed as '$have', pinned $PIN_CUDA_TOOLKIT_VERSION"; return 1; }
}

# pin_hf_snapshot HF_HOME: the pinned Qwen2-0.5B-Instruct files (pins.sh) in the offline cache layout, refs/main pointing
# at the pinned revision. A file that is already there is kept only if it is the pinned bytes; every download is
# digest-checked and retried (pin_fetch_file). Fails if any file is missing or different; never falls back to a name.
pin_hf_snapshot() {
  local home=${1:?hf home} repo snap n var entry name digest size
  repo=$home/hub/models--Qwen--Qwen2-0.5B-Instruct
  snap=$repo/snapshots/$PIN_QWEN2_REV
  pin_is_commit "$PIN_QWEN2_REV" || { pin_refuse "HF revision pin is not a 40-hex commit: '$PIN_QWEN2_REV'"; return 1; }
  mkdir -p "$snap" "$repo/refs" || return 1
  for n in 1 2 3 4 5 6 7 8; do
    var=PIN_QWEN2_FILE_$n
    entry=${!var:-}
    read -r name digest size <<<"$entry"
    [ -n "$name" ] || { pin_refuse "$var is empty"; return 1; }
    pin_verify_file "$snap/$name" "$digest" "$size" 2>/dev/null \
      || pin_fetch_file "$PIN_QWEN2_URL_BASE/$name" "$snap/$name" "$digest" "$size" || return 1
  done
  printf '%s\n' "$PIN_QWEN2_REV" > "$repo/refs/main"
}
