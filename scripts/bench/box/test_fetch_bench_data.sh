#!/bin/bash
# test_fetch_bench_data.sh — tests for fetch_bench_data.sh, no network beyond 127.0.0.1.
# A python server plays the Worker (302 to an "R2" URL on the same port, 403/404/429 per name)
# and R2 (Range-capable, refuses any request that carries an Authorization header). The real curl
# runs through a shim that only rewrites https:// to http:// and the retry delay, so redirect
# handling, resume, stdin configs and output hygiene are exercised for real.
#   bash scripts/bench/box/test_fetch_bench_data.sh
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
script="$here/fetch_bench_data.sh"
T=$(mktemp -d)
pid=''
cleanup() { [ -z "$pid" ] || kill "$pid" 2>/dev/null; rm -rf "$T"; }
trap cleanup EXIT
fails=0 passes=0
ok() { passes=$((passes + 1)); echo "ok   $1"; }
bad() { fails=$((fails + 1)); echo "FAIL $1" >&2; }
check() { # check <label> <command...>
    local label=$1; shift
    if "$@"; then ok "$label"; else bad "$label"; fi
}
has() { grep -qF -- "$2" "$1"; }
lacks() { ! grep -qF -- "$2" "$1"; }

TOKEN='tok123.SECRET-run_token'
SIG='SECRETSIG0123456789abcdef'
mkdir -p "$T/shim" "$T/data"
head -c 300000 /dev/urandom >"$T/data/obj1"
mkdir -p "$T/src/inner"
echo hello >"$T/src/inner/a.txt"
head -c 50000 /dev/urandom >"$T/src/b.bin"
tar -C "$T/src" -c . | zstd -q -o "$T/data/tarball"
obj1_sha=$(sha256sum "$T/data/obj1" | cut -d' ' -f1)
tar_sha=$(sha256sum "$T/data/tarball" | cut -d' ' -f1)
zero_sha=$(printf '0%.0s' {1..64})

cat >"$T/shim/curl" <<'EOF'
#!/bin/bash
# Test shim: loopback http instead of https, no retry delay, real curl otherwise.
args=()
while [ "$#" -gt 0 ]; do
    case $1 in
        --proto) args+=(--proto '=http'); shift ;;
        --retry-delay) args+=(--retry-delay 0); shift ;;
        *) args+=("$1") ;;
    esac
    shift
done
echo "${args[*]}" >>"$KF_SHIM_ARGS"
if printf '%s\n' "${args[@]}" | grep -qx -- '--config'; then
    sed -e 's#https://#http://#g' | /usr/bin/curl "${args[@]}"
else
    /usr/bin/curl "${args[@]}"
fi
EOF
chmod +x "$T/shim/curl"

cat >"$T/server.py" <<'EOF'
import http.server, sys, os, json
T = sys.argv[1]
log = open(os.path.join(T, 'server.log'), 'a', buffering=1)
state = {'truncated': False}
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        log.write(json.dumps({'path': self.path.split('?')[0], 'auth': 'Authorization' in self.headers,
                              'ua': self.headers.get('User-Agent', '')}) + '\n')
        port = self.server.server_address[1]
        if self.path.startswith('/p/bench-data/'):
            if self.headers.get('Authorization') != 'Bearer ' + os.environ['TOKEN']:
                return self.reply(401, b'no')
            name = self.path.split('/')[-1]
            code = {'specviewperf-15.0.0.SPEC_General_License.pdf': 403,
                    'specviewperf-15.0.0.SPECviewperf-Setup.exe': 429}.get(name)
            if code: return self.reply(code, b'{"state":"X"}')
            target = {'specviewperf-15.0.0.CHECKSUMS.txt': 'obj1', 'specviewperf-15.0.0.tar.zst': 'tarball',
                      'obj-badhash': 'obj1', 'obj-twice': 'obj1'}.get(name)
            if not target: return self.reply(404, b'no')
            self.send_response(302)
            self.send_header('Location', 'https://127.0.0.1:%d/r2/%s?X-Amz-Signature=%s' % (port, target, os.environ['SIG']))
            self.send_header('Cache-Control', 'no-store')
            self.send_header('Content-Length', '0')
            return self.end_headers()
        if self.path.startswith('/r2/'):
            if 'Authorization' in self.headers: return self.reply(400, b'two auth mechanisms')
            data = open(os.path.join(T, 'data', self.path.split('?')[0].split('/')[-1]), 'rb').read()
            rng = self.headers.get('Range')
            start = int(rng.split('=')[1].split('-')[0]) if rng else 0
            body = data[start:]
            self.send_response(206 if rng else 200)
            if rng: self.send_header('Content-Range', 'bytes %d-%d/%d' % (start, len(data) - 1, len(data)))
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            if os.path.exists(os.path.join(T, 'truncate-once')) and not state['truncated'] and not rng:
                state['truncated'] = True
                self.wfile.write(body[:100000]); self.wfile.flush(); self.connection.close(); return
            self.wfile.write(body)
            return
        self.reply(404, b'no')
    def reply(self, code, body):
        self.send_response(code); self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
s = http.server.ThreadingHTTPServer(('127.0.0.1', 0), H)
open(os.path.join(T, 'port'), 'w').write(str(s.server_address[1]))
s.serve_forever()
EOF
TOKEN=$TOKEN SIG=$SIG python3 -I "$T/server.py" "$T" &
pid=$!
for _ in $(seq 50); do [ -s "$T/port" ] && break; sleep 0.1; done
port=$(cat "$T/port")

export KF_CI_ENV_FILE=/nonexistent
export KF_BROKER_URL="https://127.0.0.1:$port/p"
export KF_RUN_ID=123-1
export KF_RUN_TOKEN=$TOKEN
export KF_BENCH_RETRY_SLEEP=0
export KF_SHIM_ARGS="$T/shim.args"
: >"$T/shim.args"
run() { # run <label-out> args... ; sets rc, output in $T/<label>.out
    local o=$1; shift
    PATH="$T/shim:$PATH" bash "$script" "$@" >"$T/$o.out" 2>&1
    rc=$?
}
nosecrets() { lacks "$T/$1.out" "$TOKEN" && lacks "$T/$1.out" "$SIG" && lacks "$T/$1.out" "X-Amz" && lacks "$T/$1.out" "127.0.0.1"; }
# The shim sits first on PATH; make it call the real curl by absolute path.
sed -i "s#/usr/bin/curl#$(command -v curl)#g" "$T/shim/curl"

# --- argument and environment checks, no request ---------------------------------------------
run d1 --dry-run specviewperf-15.0.0.tar.zst --sha256 "$obj1_sha" --extract "$T/ex"
check "dry run exits 0" test "$rc" -eq 0
check "dry run names the plan" has "$T/d1.out" "dry run, no request made"
check "dry run masks the broker" lacks "$T/d1.out" "127.0.0.1"
check "dry run prints no token" lacks "$T/d1.out" "$TOKEN"
check "dry run made no request" test ! -s "$T/server.log"
run d2 --dry-run unknown/../x --sha256 "$obj1_sha" --out "$T/o"
check "bad name refused (2)" test "$rc" -eq 2
run d3 specviewperf-15.0.0.tar.zst --sha256 abc --out "$T/o"
check "bad sha256 refused (2)" test "$rc" -eq 2
run d4 specviewperf-15.0.0.tar.zst --sha256 "$obj1_sha"
check "target required (2)" test "$rc" -eq 2
run d5 x.bin --sha256 "$obj1_sha" --extract "$T/ex"
check "--extract needs .tar.zst (2)" test "$rc" -eq 2
run d6 x.bin --sha256 "$obj1_sha" --out "$T/o" --attempts 4
check "attempts above 3 refused (2)" test "$rc" -eq 2
( KF_RUN_TOKEN='bad token' PATH="$T/shim:$PATH" bash "$script" --dry-run x.bin --sha256 "$obj1_sha" --out "$T/o" >"$T/d7.out" 2>&1 )
check "malformed token refused (2)" test "$?" -eq 2
check "malformed token is not echoed" lacks "$T/d7.out" "bad token"
( KF_BROKER_URL=http://x.example PATH="$T/shim:$PATH" bash "$script" --dry-run x.bin --sha256 "$obj1_sha" --out "$T/o" >"$T/d8.out" 2>&1 )
check "http broker refused (2)" test "$?" -eq 2

# --- file mode ----------------------------------------------------------------------------
run f1 specviewperf-15.0.0.CHECKSUMS.txt --sha256 "$obj1_sha" --out "$T/out/obj1"
check "file download exits 0" test "$rc" -eq 0
check "file content matches" cmp -s "$T/out/obj1" "$T/data/obj1"
check "no .part left" test ! -e "$T/out/obj1.part"
check "output has no token, signature or URL" nosecrets f1
check "R2 request carried no Authorization header" bash -c "! grep -F '/r2/' '$T/server.log' | grep -q '\"auth\": true'"
check "Worker request carried the token" bash -c "grep -F '/p/bench-data/' '$T/server.log' | grep -q '\"auth\": true'"
check "descriptive User-Agent" has "$T/server.log" "kayfabe-bench/1.0"
check "token never on a command line" lacks "$T/shim.args" "$TOKEN"
check "presigned URL never on a command line" lacks "$T/shim.args" "X-Amz"
check "download used resume and retry" has "$T/shim.args" "-C - --retry 5"
check "redirects are not followed" has "$T/shim.args" "--max-redirs 0"

: >"$T/server.log"
run f2 specviewperf-15.0.0.CHECKSUMS.txt --sha256 "$obj1_sha" --out "$T/out/obj1"
check "local first: present and matching makes no request" test ! -s "$T/server.log"

run f3 specviewperf-15.0.0.CHECKSUMS.txt --sha256 "$zero_sha" --out "$T/out/obj-bad"
check "hash mismatch exits 3" test "$rc" -eq 3
check "mismatch leaves no final file" test ! -e "$T/out/obj-bad"
check "mismatch deletes the .part" test ! -e "$T/out/obj-bad.part"
check "mismatch output has no secrets" nosecrets f3

touch "$T/truncate-once"
rm -f "$T/out/obj1"
run f4 specviewperf-15.0.0.CHECKSUMS.txt --sha256 "$obj1_sha" --out "$T/out/obj1"
check "interrupted download resumes and verifies" bash -c "test $rc -eq 0 && cmp -s '$T/out/obj1' '$T/data/obj1'"
rm -f "$T/truncate-once"

: >"$T/server.log"
run f5 specviewperf-15.0.0.SPEC_General_License.pdf --sha256 "$obj1_sha" --out "$T/out/lic"
check "403 refusal exits 4" test "$rc" -eq 4
check "403 message is clear" has "$T/f5.out" "refused"
check "403 makes exactly one Worker request" test "$(wc -l <"$T/server.log")" -eq 1
check "403 output has no secrets" nosecrets f5
run f6 specviewperf-15.0.0.SPECviewperf-Setup.exe --sha256 "$obj1_sha" --out "$T/out/exe"
check "URL limit (429) exits 4" test "$rc" -eq 4
run f7 nosuchobject.bin --sha256 "$obj1_sha" --out "$T/out/x"
check "unknown object (404) exits 4" test "$rc" -eq 4
( KF_RUN_TOKEN=wrong PATH="$T/shim:$PATH" bash "$script" specviewperf-15.0.0.CHECKSUMS.txt --sha256 "$obj1_sha" --out "$T/out/y" >"$T/f8.out" 2>&1 )
check "wrong token (401) exits 4" test "$?" -eq 4
check "wrong token is not echoed" lacks "$T/f8.out" "wrong"

# --- extract mode -------------------------------------------------------------------------
run e1 specviewperf-15.0.0.tar.zst --sha256 "$tar_sha" --extract "$T/ex"
check "extract exits 0" test "$rc" -eq 0
check "extracted file a.txt" has "$T/ex/inner/a.txt" hello
check "extracted file b.bin" cmp -s "$T/ex/b.bin" "$T/src/b.bin"
check "no staging dir left" test ! -e "$T/ex.part"
check "marker written" has "$T/ex/.bench-data.sha256" "$tar_sha"
check "extract output has no secrets" nosecrets e1
: >"$T/server.log"
run e2 specviewperf-15.0.0.tar.zst --sha256 "$tar_sha" --extract "$T/ex"
check "extract local first: no request" bash -c "test $rc -eq 0 && test ! -s '$T/server.log'"
run e3 specviewperf-15.0.0.tar.zst --sha256 "$zero_sha" --extract "$T/ex2"
check "extract hash mismatch exits 3" test "$rc" -eq 3
check "extract mismatch removes staging and result" bash -c "test ! -e '$T/ex2' && test ! -e '$T/ex2.part'"
mkdir -p "$T/ex3" && echo x >"$T/ex3/keep"
run e4 specviewperf-15.0.0.tar.zst --sha256 "$tar_sha" --extract "$T/ex3"
check "extract refuses a non-empty target (2)" bash -c "test $rc -eq 2 && test -f '$T/ex3/keep'"
run e5 specviewperf-15.0.0.SPEC_General_License.pdf.tar.zst --sha256 "$tar_sha" --extract "$T/ex4"
check "extract of an unknown name fails (4) and leaves nothing" bash -c "test $rc -eq 4 && test ! -e '$T/ex4' && test ! -e '$T/ex4.part'"

echo "passed $passes, failed $fails"
[ "$fails" -eq 0 ]
