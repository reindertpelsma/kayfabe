#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Parallel, checkpointed, SHA-256 pinned downloads; stdlib only, including under python3 -I.

Use URL '-' to read a URL from stdin without putting a signed locator in argv.
--worker-auth sends the run/Access capability from the environment ONLY to the original
request, never to the redirect target (even on the same host). No remote text is logged.

Every download is written to `<out>.unverified` and renamed to `<out>` only after the size and
the pinned SHA-256 both match (the pin comes from the caller, never from the server). On any
failure `<out>` does not exist. The connection is untrusted: a SIZE or HASH failure (an
oversize response, or a size/pin mismatch after the full transfer) DELETES the `.unverified`
file and its resume state; only a plain interruption (network error, timeout, kill) keeps them
so the download can resume. If `<out>` already matches the pin, nothing is downloaded. --fallback-url retries the
whole download from a second URL (fresh `.unverified`, same pin and size) after ANY failure of the
first that is not local: connect/DNS/TLS error, reset, stall, truncation, non-2xx status, malformed or
wrong-size response, pin mismatch (a bad argument or a disk error does not fall back); it never receives --worker-auth headers and never follows redirects.
"""
import argparse
import concurrent.futures
import errno
import fcntl
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

BLOCK = 1024 * 1024
ATTEMPTS = 5
try:
    # Read timeout (stall) in seconds; the override exists for tests only.
    TIMEOUT = float(os.environ.get("KF_FETCH_TIMEOUT", "30"))
    if not 0.05 <= TIMEOUT <= 3600:
        raise ValueError()
except ValueError:
    TIMEOUT = 30.0
REFRESHES = 2
REDIRECTS = (301, 302, 303, 307, 308)


class FetchError(Exception):
    def __init__(self, kind, local=False):
        self.kind = kind
        # local: our own side is at fault (arguments, disk); a fallback URL cannot help.
        self.local = local


class Expired(Exception):
    pass


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def valid_url(url, *, secure=False):
    try:
        p = urllib.parse.urlsplit(url)
        if (p.scheme not in (("https",) if secure else ("http", "https")) or not p.hostname
                or p.username or p.password or p.fragment or not p.port and p.netloc.endswith(":")):
            raise ValueError()
        if len(url) > 16384 or any(ord(c) <= 32 or ord(c) == 127 for c in url):
            raise ValueError()
    except (ValueError, TypeError):
        raise FetchError("NETWORK") from None
    return url


def unverified_path(out):
    return Path(str(out) + ".unverified")


def fsync_directory(path):
    directory = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def file_sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as source:
        while data := source.read(BLOCK):
            h.update(data)
    return h.hexdigest()


def discard_partial(out, state):
    # A fallback never mixes bytes from two sources: start from nothing.
    for path in (unverified_path(out), state):
        Path(path).unlink(missing_ok=True)


def safe_label(value):
    return re.sub(r"[^A-Za-z0-9._-]", "_", value)[:100]


def digest_fd(fd, start, length):
    h = hashlib.sha256()
    while length:
        data = os.pread(fd, min(BLOCK, length), start)
        if not data:
            raise FetchError("SIZE")
        h.update(data)
        start += len(data)
        length -= len(data)
    return h.hexdigest()


def write_at(fd, data, start):
    view = memoryview(data)
    while view:
        n = os.pwrite(fd, view, start)
        if n <= 0:
            raise FetchError("SIZE")
        view = view[n:]
        start += n


class Remote:
    def __init__(self, url, follow, headers=None):
        self.original = valid_url(url)
        self.url = self.original
        self.follow = follow
        self.headers = headers or {}
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
        self.lock = threading.Lock()
        self.generation = 0
        self.refreshes = 0
        self.redirected = False
        self.meta = None

    def open(self, url, headers):
        request = urllib.request.Request(url, headers={"Accept-Encoding": "identity", "User-Agent": "kayfabe-fetch/1.0", **headers})
        try:
            return self.opener.open(request, timeout=getattr(self, "timeout", 30))
        except urllib.error.HTTPError as e:
            if e.code in REDIRECTS or e.code == 416:
                return e
            with e:
                if e.code == 403:
                    body = e.read(4096).lower()
                    if any(word in body for word in (b"expired", b"requestexpired", b"expiry")):
                        raise Expired() from None
            raise FetchError("AUTH" if e.code in (401, 403) else ("SIZE" if e.code == 412 else "NETWORK")) from None
        except (OSError, ValueError, http.client.HTTPException):
            raise FetchError("NETWORK") from None

    def retry_open(self, url, headers):
        for attempt in range(ATTEMPTS):
            try:
                return self.open(url, headers)
            except FetchError as e:
                if e.kind != "NETWORK" or attempt + 1 == ATTEMPTS:
                    raise
                time.sleep(0.25 * 2 ** attempt)

    def probe(self):
        # GET Range also works on Worker routes that mint only on GET, not HEAD.
        # Resolve once, then retry/probe ONLY the final endpoint.
        resolve = self.open if self.follow else self.retry_open
        with resolve(self.original, {**self.headers, "Range": "bytes=0-0"}) as response:
            if response.status in REDIRECTS:
                if not self.follow:
                    raise FetchError("NETWORK")
                location = response.headers.get("Location", "")
                self.url = valid_url(urllib.parse.urljoin(self.original, location),
                                     secure=urllib.parse.urlsplit(self.original).scheme == "https")
                self.redirected = True
            else:
                self.url = self.original
                return self.metadata(response)
        with self.retry_open(self.url, {"Range": "bytes=0-0"}) as response:
            if response.status in REDIRECTS:
                raise FetchError("NETWORK")
            return self.metadata(response)

    @staticmethod
    def metadata(response):
        if response.headers.get("Content-Encoding", "identity").lower() != "identity":
            raise FetchError("SIZE")
        etag = response.headers.get("ETag")
        if response.status == 206:
            match = re.fullmatch(r"bytes 0-0/(\d+)", response.headers.get("Content-Range", ""))
            if not match or int(match[1]) <= 0:
                raise FetchError("SIZE")
            size, ranges = int(match[1]), True
        elif response.status == 200:
            length = response.headers.get("Content-Length", "")
            if not re.fullmatch(r"\d+", length):
                raise FetchError("SIZE")
            size, ranges = int(length), False
        elif response.status == 416 and response.headers.get("Content-Range") == "bytes */0":
            size, ranges = 0, True
        else:
            raise FetchError("NETWORK")
        if size > 2**63 - 1:
            raise FetchError("SIZE")
        return size, etag, ranges

    def refresh(self, generation):
        with self.lock:
            if generation != self.generation:
                return
            if not self.redirected or self.refreshes >= REFRESHES:
                raise FetchError("AUTH")
            self.refreshes += 1
            try:
                meta = self.probe()
            except Expired:
                raise FetchError("AUTH") from None
            if meta != self.meta:
                raise FetchError("SIZE")
            self.generation += 1

    def response(self, headers):
        # Read URL and generation together; a single refresh serves all expired workers.
        for _ in range(REFRESHES + 1):
            with self.lock:
                url, generation = self.url, self.generation
            try:
                # A direct authenticated URL needs its headers for each request. Redirect
                # destinations NEVER inherit them, even when both endpoints share a host.
                return self.open(url, {**({} if self.redirected else self.headers), **headers})
            except Expired:
                self.refresh(generation)
        raise FetchError("AUTH")


class Download:
    def __init__(self, url, out, sha256, streams=8, chunk_size=16 * BLOCK, state=None,
                 follow_redirect=False, origin_headers=None, expected_size=None):
        self.remote = Remote(url, follow_redirect, origin_headers)
        self.remote.timeout = TIMEOUT
        self.out = Path(out)
        self.unverified = unverified_path(out)
        self.state = Path(state) if state is not None else Path(str(out) + ".fetch-state.json")
        if self.state.resolve() in (self.out.resolve(), self.unverified.resolve()):
            raise FetchError("SIZE", local=True)
        self.pin = sha256.lower()
        if not re.fullmatch(r"[0-9a-f]{64}", self.pin) or not 1 <= streams <= 256 or chunk_size <= 0:
            raise FetchError("SIZE", local=True)
        if expected_size is not None and not 0 <= expected_size <= 2**63 - 1:
            raise FetchError("SIZE", local=True)
        self.expected_size = expected_size
        self.streams = streams
        self.chunk_size = chunk_size
        self.stop = threading.Event()
        self.lock = threading.Lock()
        self.completed = {}
        self.transferred = 0
        self.next_chunk = 0
        self.started = time.monotonic()

    def save(self):
        # Called under the completion lock: data reaches disk before the atomic checkpoint.
        os.fsync(self.fd)
        temporary = self.state.with_name(self.state.name + ".tmp")
        with temporary.open("w", encoding="utf-8") as output:
            os.chmod(temporary, 0o600)
            json.dump({**self.identity, "completed": self.completed}, output, sort_keys=True)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, self.state)
        fsync_directory(self.state.parent)

    def existing_matches(self):
        # A finished <out> that already matches the pin needs no request.
        try:
            if not self.out.is_file() or self.expected_size is not None and self.out.stat().st_size != self.expected_size:
                return False
            return file_sha256(self.out) == self.pin
        except OSError:
            return False

    def publish(self):
        # Only called after the size and the pin matched: data, then rename, then directory.
        os.fsync(self.fd)
        os.replace(self.unverified, self.out)
        fsync_directory(self.out.parent)

    def restore(self, old_size):
        try:
            with self.state.open(encoding="utf-8") as source:
                saved = json.load(source)
            if not isinstance(saved, dict) or any(saved.get(k) != v for k, v in self.identity.items()):
                return
            entries = saved["completed"]
            if not isinstance(entries, dict):
                return
        except (OSError, ValueError, TypeError, KeyError):
            return
        for key, sha in entries.items():
            if not re.fullmatch(r"0|[1-9]\d{0,18}", key):
                continue
            index = int(key)
            start = index * self.chunk_size
            end = min(start + self.chunk_size, self.size)
            if not (0 <= start < end <= old_size):
                continue
            if digest_fd(self.fd, start, end - start) == sha:
                self.completed[key] = sha

    def progress(self):
        seconds = max(time.monotonic() - self.started, 0.000001)
        done = sum(min(self.chunk_size, self.size - int(i) * self.chunk_size) for i in self.completed)
        print(f"FETCH_PROGRESS bytes={done} total={self.size} MBps={self.transferred / seconds / 1e6:.3f}", flush=True)

    def chunk(self, index):
        start = index * self.chunk_size
        end = min(start + self.chunk_size, self.size) - 1
        for attempt in range(ATTEMPTS):
            if self.stop.is_set():
                return None
            try:
                headers = {"Range": f"bytes={start}-{end}"} if self.ranges else {}
                if self.etag and not self.etag.startswith("W/"):
                    headers["If-Match"] = self.etag
                with self.remote.response(headers) as response:
                    expected_status = 206 if self.ranges else 200
                    expected_range = f"bytes {start}-{end}/{self.size}"
                    if (response.status != expected_status or
                            (self.ranges and response.headers.get("Content-Range") != expected_range) or
                            response.headers.get("Content-Encoding", "identity").lower() != "identity" or
                            response.headers.get("ETag") != self.etag):
                        raise FetchError("SIZE")
                    length = response.headers.get("Content-Length")
                    if length is not None and length != str(end - start + 1):
                        raise FetchError("SIZE")
                    h, offset = hashlib.sha256(), start
                    while offset <= end:
                        if self.stop.is_set():
                            return None
                        data = response.read(min(BLOCK, end + 1 - offset))
                        if not data:
                            raise FetchError("NETWORK")
                        try:
                            write_at(self.fd, data, offset)
                        except OSError:
                            raise FetchError("SIZE", local=True) from None
                        h.update(data)
                        offset += len(data)
                    if response.read(1):
                        raise FetchError("SIZE")
                return h.hexdigest()
            except (OSError, http.client.HTTPException):
                error = FetchError("NETWORK")
            except FetchError as e:
                error = e
            if error.kind != "NETWORK" or attempt + 1 == ATTEMPTS:
                raise error
            if self.stop.wait(0.25 * 2 ** attempt):
                return None
        raise FetchError("NETWORK")

    def worker(self):
        try:
            while not self.stop.is_set():
                with self.lock:
                    while str(self.next_chunk) in self.completed:
                        self.next_chunk += 1
                    index = self.next_chunk
                    self.next_chunk += 1
                if index >= self.count:
                    return
                sha = self.chunk(index)
                if sha is None:
                    return
                with self.lock:
                    self.completed[str(index)] = sha
                    self.transferred += min(self.chunk_size, self.size - index * self.chunk_size)
                    self.save()
                    self.progress()
        except BaseException:
            self.stop.set()
            raise

    def run(self):
        try:
            self.transfer()
        except FetchError as e:
            if e.kind in ("SIZE", "HASH") and not e.local:
                discard_partial(self.out, self.state)
            raise

    def transfer(self):
        if self.existing_matches():
            print("FETCH_DONE secs=0.000 MBps=0.000 streams=0", flush=True)
            return
        # One process per output AND per state path. Locks are kernel-owned and survive
        # SIGKILL as unlocked files, so a killed run needs no stale-lock workaround.
        self.state.parent.mkdir(parents=True, exist_ok=True)
        self.out.parent.mkdir(parents=True, exist_ok=True)
        output_fd = os.open(self.unverified, os.O_RDWR | os.O_CREAT, 0o600)
        with os.fdopen(output_fd, "r+b") as output, open(str(self.state) + ".lock", "a+b") as state_lock:
            self.fd = output.fileno()
            fcntl.flock(state_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            fcntl.flock(self.fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            # A stale final file is never left in place while (or after) a download fails.
            self.out.unlink(missing_ok=True)
            try:
                self.remote.meta = self.remote.probe()
            except Expired:
                raise FetchError("AUTH") from None
            self.size, self.etag, self.ranges = self.remote.meta
            if self.expected_size is not None and self.size != self.expected_size:
                raise FetchError("SIZE")
            if not self.ranges:
                self.streams = 1
                self.chunk_size = max(1, self.size)
            self.count = (self.size + self.chunk_size - 1) // self.chunk_size
            self.identity = {"version": 1, "size": self.size, "etag": self.etag,
                             "chunk_size": self.chunk_size, "sha256": self.pin,
                             "target": os.path.abspath(self.unverified),
                             "source": hashlib.sha256(urllib.parse.urlunsplit(urllib.parse.urlsplit(self.remote.original)._replace(query="")).encode()).hexdigest()}
            label = urllib.parse.urlsplit(self.remote.url)
            print(f"FETCH_SOURCE host={safe_label(label.hostname)} object={safe_label(label.path.rsplit('/', 1)[-1])}", flush=True)
            old_size = os.fstat(self.fd).st_size
            self.restore(old_size)
            # Bounds must use old_size above: preallocation must not legitimise holes.
            if not self.completed:
                os.ftruncate(self.fd, 0)
            os.ftruncate(self.fd, self.size)
            if self.size and hasattr(os, "posix_fallocate"):
                try:
                    os.posix_fallocate(self.fd, 0, self.size)
                except OSError as e:
                    if e.errno not in (errno.EOPNOTSUPP, errno.ENOSYS, errno.EINVAL):
                        raise
            self.save()
            self.progress()
            # Only N tasks/futures, not one future per chunk of a 100 GB object.
            with concurrent.futures.ThreadPoolExecutor(max_workers=self.streams) as pool:
                futures = [pool.submit(self.worker) for _ in range(self.streams)]
                try:
                    for future in concurrent.futures.as_completed(futures):
                        future.result()
                finally:
                    self.stop.set()
            if os.fstat(self.fd).st_size != self.size or len(self.completed) != self.count:
                raise FetchError("SIZE")
            if digest_fd(self.fd, 0, self.size) != self.pin:
                # run() deletes the unverified bytes and state; nothing corrupt is ever resumed.
                print("FETCH_BAD_HASH", flush=True)
                raise FetchError("HASH")
            self.publish()
        seconds = max(time.monotonic() - self.started, 0.000001)
        print(f"FETCH_DONE secs={seconds:.3f} MBps={self.transferred / seconds / 1e6:.3f} streams={self.streams}", flush=True)


class Parser(argparse.ArgumentParser):
    def error(self, message):
        # argparse's normal message may echo a URL or an unrecognised signed option.
        self.exit(2, "FETCH_FAIL class=SIZE\n")


def main(argv=None):
    parser = Parser(description=__doc__)
    parser.add_argument("url", help="URL or '-' to read one URL line from stdin")
    parser.add_argument("out")
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--streams", type=int, default=8)
    parser.add_argument("--chunk-mb", type=int, default=16)
    parser.add_argument("--state")
    parser.add_argument("--size", type=int, help="optional pinned byte count, checked before allocation")
    parser.add_argument("--follow-redirect", action="store_true")
    parser.add_argument("--fallback-url", help="retried whole after a NETWORK or HASH failure of the first URL; no auth headers, no redirects")
    parser.add_argument("--worker-auth", action="store_true", help="origin-only run/Access headers from environment")
    args = parser.parse_args(argv)
    try:
        import sys
        url = sys.stdin.readline(16386).rstrip("\n") if args.url == "-" else args.url
        headers = {}
        if args.worker_auth:
            token = os.environ.get("KF_RUN_TOKEN", "")
            ident, secret = os.environ.get("KF_ACCESS_ID", ""), os.environ.get("KF_ACCESS_SECRET", "")
            if not token or bool(ident) != bool(secret) or any(not re.fullmatch(r"[A-Za-z0-9._-]+", v) for v in (token, ident, secret) if v):
                raise FetchError("AUTH")
            headers["Authorization"] = "Bearer " + token
            if ident:
                headers.update({"CF-Access-Client-Id": ident, "CF-Access-Client-Secret": secret})
        def download(source, follow, origin_headers):
            return Download(source, args.out, args.sha256, args.streams, args.chunk_mb * BLOCK,
                            args.state, follow, origin_headers, expected_size=args.size)
        try:
            download(url, args.follow_redirect, headers).run()
        except FetchError as e:
            if args.fallback_url is None or e.local:
                raise
            print("FETCH_FALLBACK reason=" + e.kind, flush=True)
            discard_partial(args.out, Path(args.state) if args.state is not None else Path(args.out + ".fetch-state.json"))
            # The fallback is a different server: it never sees the Worker credentials.
            download(args.fallback_url, False, {}).run()
        return 0
    except FetchError as e:
        print("FETCH_FAIL class=" + e.kind, flush=True)
        return {"NETWORK": 1, "HASH": 3, "SIZE": 2, "AUTH": 4}[e.kind]
    except (OSError, ValueError, TypeError, OverflowError):
        print("FETCH_FAIL class=SIZE", flush=True)
        return 2
    except Exception:
        # HTTP parser errors and remote-controlled headers must never produce a traceback.
        print("FETCH_FAIL class=NETWORK", flush=True)
        return 1
    except KeyboardInterrupt:
        print("FETCH_FAIL class=NETWORK", flush=True)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
