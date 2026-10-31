#!/bin/bash
# fetch_bench_data.sh — fetch one licensed benchmark object from the private R2 copy through the
# Vast CI Worker, using only this run's token. Runs ON a rented box (docs/DOWNLOADS.md,
# "SPECviewperf private copy: handling rules"; ci/vast-broker/DEPLOY.md, "Benchmark copy in R2").
#
#   fetch_bench_data.sh [--dry-run] [--attempts N] <name> --sha256 <hex> --out <file>
#   fetch_bench_data.sh [--dry-run] [--attempts N] <name> --sha256 <hex> --extract <dir>
#
# <name> is one of the Worker's fixed names (e.g. specviewperf-15.0.0.tar.zst); the Worker answers
# an unknown name with 404. --sha256 is the pin the CALLER holds (the repo pins it; the Worker does
# not); a mismatch deletes the bytes and exits 3. --out downloads to <file>.part with resume
# (-C -) and renames after the hash check. --extract streams `curl | zstd -d | tar -x` into
# <dir>.part (no 25 GB temporary file), hashes the compressed stream on the way, and renames to
# <dir> only after the hash matches; it cannot resume, so a failed stream restarts from zero.
#
# Environment (the box's /run/kf-ci/env is read when readable, as kf-box-blob-url does):
#   KF_BROKER_URL (or WORKER_URL)  https origin plus the secret path prefix, no trailing slash
#   KF_RUN_ID                      <run>-<attempt>
#   KF_RUN_TOKEN                   the run token
#   KF_ACCESS_ID/KF_ACCESS_SECRET  optional Cloudflare Access pair, both or none
#
# Secrecy: the token and the presigned URL never appear on a command line, in the output or in a
# log. Step 1 asks the Worker with --max-redirs 0 and captures the 302 target with
# `-w '%{redirect_url}'` into a shell variable; step 2 downloads with the URL given to curl on stdin
# (--config -). The Authorization header is never sent to R2 (R2 refuses a request that carries both
# a header and a query signature). curl diagnostics are filtered so no URL is echoed. Each attempt
# asks the Worker for a fresh URL (valid at most 20 minutes, never past the run's deadline); the
# Worker allows 4 per object per run, so --attempts is capped at 3.
# --dry-run validates the arguments and the environment, prints the plan with the broker masked,
# and makes no request.
set -euo pipefail
umask 077

die() { echo "fetch_bench_data: $*" >&2; exit "${2:-2}"; }
usage() { sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//' >&2; exit 2; }

dry=0 attempts=3 name='' want='' out='' dir=''
while [ "$#" -gt 0 ]; do
    case $1 in
        --dry-run) dry=1 ;;
        --attempts|--sha256|--out|--extract)
            [ "$#" -ge 2 ] || die "$1 needs a value"
            case $1 in
                --attempts) attempts=$2 ;;
                --sha256) want=$2 ;;
                --out) out=$2 ;;
                --extract) dir=$2 ;;
            esac
            shift ;;
        -h|--help) usage ;;
        -*) die "unknown option $1" ;;
        *) [ -z "$name" ] || die "one object name only"; name=$1 ;;
    esac
    shift
done
[ -n "$name" ] || usage
case $name in ''|.|..|*[!A-Za-z0-9._-]*) die "invalid object name" ;; esac
[ "${#name}" -le 100 ] || die "invalid object name"
case $want in *[!0-9a-f]*|'') die "--sha256 must be 64 lowercase hex digits" ;; esac
[ "${#want}" -eq 64 ] || die "--sha256 must be 64 lowercase hex digits"
case $attempts in 1|2|3) ;; *) die "--attempts must be 1, 2 or 3 (the Worker allows 4 URLs per object per run)" ;; esac
if [ -n "$out" ] && [ -n "$dir" ]; then die "--out and --extract are exclusive"; fi
[ -n "$out" ] || [ -n "$dir" ] || die "one of --out or --extract is required"
if [ -n "$dir" ]; then
    case $name in *.tar.zst) ;; *) die "--extract needs a .tar.zst object" ;; esac
fi

env_file=${KF_CI_ENV_FILE:-/run/kf-ci/env}
# shellcheck disable=SC1090
if [ -r "$env_file" ]; then . "$env_file"; fi
broker=${KF_BROKER_URL:-${WORKER_URL:-}}
broker=${broker%/}
run=${KF_RUN_ID:-}
token=${KF_RUN_TOKEN:-}
case $broker in https://?*) ;; *) die "KF_BROKER_URL (or WORKER_URL) must be an https URL" ;; esac
case $broker in *[!A-Za-z0-9._:/@~-]*) die "KF_BROKER_URL has unexpected characters" ;; esac
case $run in ''|*[!0-9-]*|-*|*-) die "KF_RUN_ID must be <run>-<attempt>" ;; esac
case $token in ''|*[!A-Za-z0-9._-]*) die "KF_RUN_TOKEN missing or malformed" ;; esac
case ${KF_ACCESS_ID:-}${KF_ACCESS_SECRET:-} in *[!A-Za-z0-9._-]*) die "KF_ACCESS_* malformed" ;; esac
if [ -n "${KF_ACCESS_ID:-}" ] || [ -n "${KF_ACCESS_SECRET:-}" ]; then
    if [ -z "${KF_ACCESS_ID:-}" ] || [ -z "${KF_ACCESS_SECRET:-}" ]; then
        die "KF_ACCESS_ID and KF_ACCESS_SECRET: both or none"
    fi
fi
ua='kayfabe-bench/1.0 (+https://github.com/reindertpelsma/kayfabe)'
mode='file'
if [ -n "$dir" ]; then mode='extract'; fi

if [ "$dry" = 1 ]; then
    echo "fetch_bench_data: dry run, no request made"
    echo "  object   $name"
    echo "  route    <broker>/bench-data/$run/$name"
    echo "  mode     $mode"
    echo "  target   ${out:-$dir}"
    echo "  sha256   $want"
    echo "  attempts $attempts"
    exit 0
fi

command -v curl >/dev/null || die "curl not found"
command -v sha256sum >/dev/null || die "sha256sum not found"
if [ "$mode" = extract ]; then
    command -v zstd >/dev/null || die "zstd not found"
    command -v tar >/dev/null || die "tar not found"
    command -v mkfifo >/dev/null || die "mkfifo not found"
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
errs="$work/err"

hash_of() { sha256sum -- "$1" | cut -d' ' -f1; }
# Strip every URL out of anything we are about to show.
scrub() { sed -E 's#[A-Za-z][A-Za-z0-9+.-]*://[^[:space:]"]*#<url>#g'; }
show_err() {
    if [ -s "$errs" ]; then head -n 3 "$errs" | scrub >&2; fi
    : >"$errs"
}

headers() {
    printf 'header = "Authorization: Bearer %s"\n' "$token"
    [ -z "${KF_ACCESS_ID:-}" ] || printf 'header = "CF-Access-Client-Id: %s"\n' "$KF_ACCESS_ID"
    [ -z "${KF_ACCESS_SECRET:-}" ] || printf 'header = "CF-Access-Client-Secret: %s"\n' "$KF_ACCESS_SECRET"
}

# Step 1: ask the Worker. Sets $location (secret; never printed) or returns non-zero.
location=''
ask_worker() {
    local reply code rc=0
    reply=$({
        headers
        printf 'url = "%s/bench-data/%s/%s"\n' "$broker" "$run" "$name"
    } | curl --config - --silent --max-time 30 --max-redirs 0 --proto '=https' --noproxy '*' \
        --user-agent "$ua" --output /dev/null --write-out '%{http_code} %{redirect_url}' \
        2>/dev/null) || rc=$?
    if [ "$rc" -ne 0 ]; then
        echo "fetch_bench_data: the Worker request failed (curl exit $rc)" >&2
        return 1
    fi
    code=${reply%% *}
    case $code in
        302) location=${reply#* } ;;
        401) echo "fetch_bench_data: Worker refused the run token (401): run ended or token invalid" >&2; return 4 ;;
        403) echo "fetch_bench_data: Worker refused the request (403)" >&2; return 4 ;;
        404) echo "fetch_bench_data: Worker does not know object $name (404)" >&2; return 4 ;;
        429) echo "fetch_bench_data: Worker URL limit for $name reached for this run (429)" >&2; return 4 ;;
        *) echo "fetch_bench_data: unexpected Worker answer (HTTP $code)" >&2; return 1 ;;
    esac
    case $location in https://?*) ;; *) echo "fetch_bench_data: Worker redirect missing or not https" >&2; return 1 ;; esac
    case $location in *[\"\\]*|*[[:space:]]*) echo "fetch_bench_data: Worker redirect malformed" >&2; return 1 ;; esac
    return 0
}

# Step 2 (file): resumable download into $1.
download_file() {
    local rc=0
    printf 'url = "%s"\n' "$location" | curl --config - --fail --silent --show-error \
        --proto '=https' --noproxy '*' --max-redirs 0 --user-agent "$ua" \
        -C - --retry 5 --retry-delay 5 --connect-timeout 30 --speed-limit 1024 --speed-time 120 \
        --output "$1" 2>"$errs" || rc=$?
    [ "$rc" -eq 0 ] || { show_err; echo "fetch_bench_data: download failed (curl exit $rc)" >&2; }
    return "$rc"
}

# Step 2 (stream): curl | tee fifo | zstd -d | tar -x. Sets $streamhash.
streamhash=''
download_stream() {
    local stage=$1 fifo="$work/fifo" shpid status
    mkfifo "$fifo"
    sha256sum <"$fifo" | cut -d' ' -f1 >"$work/stream.sha256" &
    shpid=$!
    set +e
    printf 'url = "%s"\n' "$location" | curl --config - --fail --silent --show-error \
        --proto '=https' --noproxy '*' --max-redirs 0 --user-agent "$ua" \
        --connect-timeout 30 --speed-limit 1024 --speed-time 120 --output - 2>"$errs" \
        | tee "$fifo" | zstd -d -c --long=31 | tar -x -C "$stage" --no-same-owner
    status=("${PIPESTATUS[@]}")
    set -e
    wait "$shpid" || true
    streamhash=$(cat "$work/stream.sha256" 2>/dev/null || true)
    if [ "${status[0]}" -ne 0 ] || [ "${status[1]}" -ne 0 ] || [ "${status[2]}" -ne 0 ] || [ "${status[3]}" -ne 0 ]; then
        show_err
        echo "fetch_bench_data: stream failed (curl ${status[0]}, tee ${status[1]}, zstd ${status[2]}, tar ${status[3]})" >&2
        return 1
    fi
}

attempt=0
if [ "$mode" = file ]; then
    part="$out.part"
    # Local first: a finished copy that matches the pin needs no request.
    if [ -f "$out" ] && [ "$(hash_of "$out")" = "$want" ]; then
        echo "fetch_bench_data: $out already present and matches the pin"
        exit 0
    fi
    mkdir -p -- "$(dirname -- "$out")"
    while [ "$attempt" -lt "$attempts" ]; do
        attempt=$((attempt + 1))
        rc=0
        ask_worker || rc=$?
        if [ "$rc" -eq 4 ]; then exit 4; fi
        if [ "$rc" -eq 0 ]; then
            rc=0; download_file "$part" || rc=$?
            location=''
            if [ "$rc" -eq 0 ]; then break; fi
            # A resumed .part that is already complete makes curl fail with a range error.
            if [ -f "$part" ] && [ "$(hash_of "$part")" = "$want" ]; then rc=0; break; fi
        fi
        [ "$attempt" -lt "$attempts" ] || { echo "fetch_bench_data: giving up after $attempt attempt(s)" >&2; exit 1; }
        sleep "${KF_BENCH_RETRY_SLEEP:-10}"
    done
    got=$(hash_of "$part")
    if [ "$got" != "$want" ]; then
        rm -f -- "$part"
        echo "fetch_bench_data: sha256 mismatch for $name (got $got, pinned $want); bytes deleted" >&2
        exit 3
    fi
    mv -f -- "$part" "$out"
    echo "fetch_bench_data: $name -> $out (sha256 ok)"
    exit 0
fi

# extract mode
marker=.bench-data.sha256
if [ -f "$dir/$marker" ] && [ "$(cat "$dir/$marker")" = "$want" ]; then
    echo "fetch_bench_data: $dir already extracted from the pinned archive"
    exit 0
fi
if [ -e "$dir" ] && [ -n "$(ls -A -- "$dir" 2>/dev/null)" ]; then
    die "$dir exists and is not empty; refusing to extract into it"
fi
stage="$dir.part"
while [ "$attempt" -lt "$attempts" ]; do
    attempt=$((attempt + 1))
    rm -rf -- "$stage"
    mkdir -p -- "$stage"
    rc=0
    ask_worker || rc=$?
    if [ "$rc" -eq 4 ]; then rm -rf -- "$stage"; exit 4; fi
    if [ "$rc" -eq 0 ]; then
        rc=0; download_stream "$stage" || rc=$?
        location=''
        if [ "$rc" -eq 0 ]; then break; fi
    fi
    rm -rf -- "$stage"
    [ "$attempt" -lt "$attempts" ] || { echo "fetch_bench_data: giving up after $attempt attempt(s)" >&2; exit 1; }
    sleep "${KF_BENCH_RETRY_SLEEP:-10}"
done
if [ "$streamhash" != "$want" ]; then
    rm -rf -- "$stage"
    echo "fetch_bench_data: sha256 mismatch for $name (got $streamhash, pinned $want); extracted bytes deleted" >&2
    exit 3
fi
printf '%s\n' "$want" >"$stage/$marker"
rmdir -- "$dir" 2>/dev/null || true
mv -- "$stage" "$dir"
echo "fetch_bench_data: $name -> $dir (sha256 ok)"
