#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Manifest-driven parallel downloader for a box or a standalone machine (stdlib only, Python 3.9+).

STATUS: LIVE, 2026-11-15 (step 3 of docs/design/V3_BOX_DOWNLOADER_AND_SUPPLY_CHAIN.md; GPU-free tests only, no box
has run it). Built on scripts/ci/kf_fetch.py: every transfer is `Download` (`<dest>.unverified`, per-chunk
checkpoints, Range resume that re-verifies each saved chunk, size and sha256 checked before an atomic rename,
the partial deleted on a size or hash violation, no capability header sent to a redirect target).

    box_downloader.py fetch MANIFEST [--dest DIR] [--only NAME...] [--stages A:B] [--jobs N] [--streams N]
                      [--limit-bandwidth MBPS] [--stream-limit MBPS] [--pin NAME=PATH] [--pin-dir DIR]
                      [--urls FILE] [--s3-config FILE] [--status FILE] [--no-nice]
    box_downloader.py wait [--dest DIR | --status FILE] [--deadline SECS] ITEM...
    box_downloader.py status [--dest DIR | --status FILE]
    box_downloader.py verify MANIFEST [--dest DIR] [--only NAME...]
    box_downloader.py check MANIFEST

MANIFEST (JSON, schema `kayfabe.download-manifest/1`, at most 1 MiB, strict: unknown keys, duplicate keys,
NaN and wrong types are refused; it is validated once, here):
    {"schema": "kayfabe.download-manifest/1",
     "stages": ["provision", "apps"],                      # first-use order; needed_by names one of them
     "items": [{"name": "x.tar", "dest": "dl/x.tar",       # dest: relative to --dest, no .., no symlink on the way
                "sha256": "<64 hex>", "size": 123,          # both pinned HERE, never taken from a server
                "needed_by": "apps", "priority": 0,         # order: stage index, then priority, then manifest order
                "local": "/abs/path/x.tar",                 # optional pin, used only if size and sha256 match
                "sources": [{"kind": "signed", "ref": "x.tar"},          # URL from the --urls file (presigned)
                            {"kind": "s3", "bucket": "b", "key": "x.tar"}, # S3-compatible, creds from env/config
                            {"kind": "mirror", "url": "https://..."}]}]}  # plain https, optional follow_redirect
Source order per item is the fallback order; signed and s3 (private) must precede mirrors. A source that fails for
a remote reason (network, auth, size, hash) falls through to the next with a fresh `.unverified`; a local fault
(bad path, disk) fails the item at once. Items are claimed in (stage, priority) order by N workers.

Secrets: URLs and credentials are never logged (every output line is built from validated names and numbers and
passed through a URL scrubber). The tool reads no run token and no broker variable: private sources come from a
presigned-URL JSON file (`--urls`, `kayfabe.download-urls/1`, mode 0600, re-read on every attempt so a bootstrap
layer can refresh it) or from S3 credentials (env AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY / AWS_SESSION_TOKEN /
KF_S3_ENDPOINT / KF_S3_REGION, or `--s3-config FILE` mode 0600 with endpoint, region, access_key_id,
secret_access_key). The only Worker-aware code is scripts/ci/box_downloader_worker.py (bootstrap layer).

Bandwidth: `--limit-bandwidth` is a global cap in MB/s (1e6 bytes/s), `--stream-limit` a cap per connection; 0 = off.
DEFAULT_LIMIT_MBPS is a DESIGN value, unmeasured. Output: one `KF_STEP name=download.<item> secs= bytes= source=
MBps=` line per completed item (source is local, r2/signed, s3 or mirror<N>, never a URL), `KF_DOWNLOAD_FAIL ...
class=RETRY` per failed item, and a bounded JSON status file (`<dest>/.kf-downloader/status.json`).
Exit: 0 all verified, 1 an item failed (harness RETRY class, never a guest FAIL), 2 usage/manifest/disk,
3 wait deadline, 4 downloader gone, 5 cancelled (SIGTERM/SIGINT: partials kept for resume).
"""
import argparse
import dataclasses
import datetime
import fcntl
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import sys
import threading
import time
import urllib.parse

sys.path.insert(0, str(Path(__file__).resolve().parent))
import kf_fetch  # noqa: E402

sys.path.pop(0)

SCHEMA = "kayfabe.download-manifest/1"
URLS_SCHEMA = "kayfabe.download-urls/1"
STATUS_SCHEMA = "kayfabe.download-status/1"
MAX_MANIFEST_BYTES = 1 << 20
MAX_ITEMS = 256
MAX_SOURCES = 8
MAX_SIZE = 1 << 40
DEFAULT_LIMIT_MBPS = 125.0  # DESIGN, not measured on any box
DEFAULT_JOBS = 3
DEFAULT_STREAMS = 4
NAME_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,99}")
SEG_RE = re.compile(r"[A-Za-z0-9._@+-]{1,128}")
STATE_DIR = ".kf-downloader"
RESERVED_SUFFIXES = (".unverified", ".fetch-state.json", ".lock", ".tmp")
URL_RE = re.compile(r"[A-Za-z][A-Za-z0-9+.-]*://\S*")


class ManifestError(Exception):
    """Malformed manifest. The message names a path in the document, never a value."""


class ProviderError(Exception):
    pass


def redact(text):
    return URL_RE.sub("<url>", str(text))


# ---------------------------------------------------------------- manifest

@dataclasses.dataclass(frozen=True)
class Source:
    kind: str
    ref: str = ""
    url: str = ""
    follow_redirect: bool = False
    bucket: str = ""
    key: str = ""


@dataclasses.dataclass(frozen=True)
class Item:
    name: str
    dest: str
    sha256: str
    size: int
    sources: tuple
    needed_by: str
    priority: int
    local: object


@dataclasses.dataclass(frozen=True)
class Manifest:
    stages: tuple
    items: tuple
    sha256: str

    def ordered(self, only=(), stages=()):
        pick = [i for i in self.items if (not only or i.name in only) and (not stages or i.needed_by in stages)]
        index = {s: n for n, s in enumerate(self.stages)}
        order = {i.name: n for n, i in enumerate(self.items)}
        return sorted(pick, key=lambda i: (index[i.needed_by], i.priority, order[i.name]))


def _pairs(pairs):
    out = {}
    for k, v in pairs:
        if k in out:
            raise ManifestError("duplicate key")
        out[k] = v
    return out


def _const(name):
    raise ManifestError("non-finite number")


def _keys(doc, path, required, optional=()):
    if not isinstance(doc, dict):
        raise ManifestError(path + " is not an object")
    if set(doc) - set(required) - set(optional) or any(k not in doc for k in required):
        raise ManifestError(path + " keys")


def _int(value, path, lo, hi):
    if type(value) is not int or not lo <= value <= hi:
        raise ManifestError(path)
    return value


def _str(value, path, pattern=None, limit=2048):
    if type(value) is not str or not value or len(value) > limit or any(ord(c) < 32 or ord(c) == 127 for c in value):
        raise ManifestError(path)
    if pattern is not None and not pattern.fullmatch(value):
        raise ManifestError(path)
    return value


def valid_dest(dest):
    if type(dest) is not str or not dest or len(dest) > 512 or dest.startswith("/") or "\\" in dest:
        return False
    parts = dest.split("/")
    if any(p in ("", ".", "..") or not SEG_RE.fullmatch(p) for p in parts):
        return False
    return not parts[0] == STATE_DIR and not parts[-1].endswith(RESERVED_SUFFIXES)


def _source(doc, path, allow_http):
    if not isinstance(doc, dict) or doc.get("kind") not in ("signed", "s3", "mirror"):
        raise ManifestError(path + ".kind")
    kind = doc["kind"]
    if kind == "signed":
        _keys(doc, path, ("kind", "ref"))
        return Source("signed", ref=_str(doc["ref"], path + ".ref", NAME_RE, 100))
    if kind == "s3":
        _keys(doc, path, ("kind", "bucket", "key"))
        bucket = _str(doc["bucket"], path + ".bucket", re.compile(r"[a-z0-9][a-z0-9.-]{2,62}"), 63)
        key = _str(doc["key"], path + ".key", re.compile(r"[A-Za-z0-9._/@+=-]{1,512}"), 512)
        if any(p in ("", ".", "..") for p in key.split("/")):
            raise ManifestError(path + ".key")
        return Source("s3", bucket=bucket, key=key)
    _keys(doc, path, ("kind", "url"), ("follow_redirect",))
    url = _str(doc["url"], path + ".url")
    follow = doc.get("follow_redirect", False)
    if type(follow) is not bool:
        raise ManifestError(path + ".follow_redirect")
    try:
        kf_fetch.valid_url(url, secure=not allow_http)
    except kf_fetch.FetchError:
        raise ManifestError(path + ".url") from None
    return Source("mirror", url=url, follow_redirect=follow)


def from_document(doc, digest="", allow_http=False):
    _keys(doc, "manifest", ("schema", "stages", "items"))
    if doc["schema"] != SCHEMA:
        raise ManifestError("schema")
    stages = doc["stages"]
    if not isinstance(stages, list) or not 1 <= len(stages) <= 32:
        raise ManifestError("stages")
    stages = tuple(_str(s, "stages[]", NAME_RE, 64) for s in stages)
    if len(set(stages)) != len(stages):
        raise ManifestError("stages duplicate")
    raw_items = doc["items"]
    if not isinstance(raw_items, list) or not 1 <= len(raw_items) <= MAX_ITEMS:
        raise ManifestError("items")
    items, names, dests = [], set(), []
    for n, raw in enumerate(raw_items):
        path = f"items[{n}]"
        _keys(raw, path, ("name", "dest", "sha256", "size", "sources", "needed_by"), ("priority", "local"))
        name = _str(raw["name"], path + ".name", NAME_RE, 100)
        if name in names:
            raise ManifestError(path + ".name duplicate")
        names.add(name)
        dest = raw["dest"]
        if not valid_dest(dest):
            raise ManifestError(path + ".dest")
        folded = dest.lower()
        for other in dests:
            if folded == other or folded.startswith(other + "/") or other.startswith(folded + "/"):
                raise ManifestError(path + ".dest duplicate")
        dests.append(folded)
        sha = raw["sha256"]
        if type(sha) is not str or not re.fullmatch(r"[0-9a-f]{64}", sha):
            raise ManifestError(path + ".sha256")
        size = _int(raw["size"], path + ".size", 1, MAX_SIZE)
        if raw["needed_by"] not in stages:
            raise ManifestError(path + ".needed_by")
        srcs = raw["sources"]
        if not isinstance(srcs, list) or not 1 <= len(srcs) <= MAX_SOURCES:
            raise ManifestError(path + ".sources")
        sources = tuple(_source(s, f"{path}.sources[{k}]", allow_http) for k, s in enumerate(srcs))
        kinds = [s.kind for s in sources]
        if "mirror" in kinds and any(k != "mirror" for k in kinds[kinds.index("mirror"):]):
            raise ManifestError(path + ".sources order")
        local = raw.get("local")
        if local is not None:
            _str(local, path + ".local", limit=4096)
            if not local.startswith("/") or "\0" in local:
                raise ManifestError(path + ".local")
        items.append(Item(name, dest, sha, size, sources, raw["needed_by"], _int(raw.get("priority", 0), path + ".priority", -1000, 1000), local))
    return Manifest(stages, tuple(items), digest)


def parse_manifest(raw, allow_http=False):
    if len(raw) > MAX_MANIFEST_BYTES:
        raise ManifestError("too large")
    try:
        doc = json.loads(raw.decode("utf-8"), object_pairs_hook=_pairs, parse_constant=_const)
    except (ValueError, UnicodeError, RecursionError):
        raise ManifestError("not JSON") from None
    return from_document(doc, hashlib.sha256(raw).hexdigest(), allow_http)


def load_manifest(path, allow_http=False):
    try:
        with open(path, "rb") as f:
            raw = f.read(MAX_MANIFEST_BYTES + 1)
    except OSError:
        raise ManifestError("unreadable") from None
    return parse_manifest(raw, allow_http)


# ---------------------------------------------------------------- bandwidth

class Bucket:
    """Reservation limiter: each call takes the next slot of n/rate seconds. Average rate holds; no burst credit."""

    def __init__(self, rate, clock=time.monotonic):
        self.rate = float(rate)
        self.clock = clock
        self.next = 0.0
        self.lock = threading.Lock()

    def acquire(self, n, cancel):
        if self.rate <= 0:
            return
        with self.lock:
            now = self.clock()
            start = max(now, self.next)
            self.next = start + n / self.rate
        # wait for the END of the slot: the bytes of this read are paid for before the next one starts
        finish = start + n / self.rate
        if finish > now:
            cancel.wait(finish - now)


class Throttle:
    def __init__(self, global_rate, stream_rate, cancel):
        self.global_bucket = Bucket(global_rate)
        self.stream_rate = stream_rate
        self.cancel = cancel
        self.local = threading.local()

    def __call__(self, n):
        self.global_bucket.acquire(n, self.cancel)
        if self.stream_rate > 0:
            bucket = getattr(self.local, "bucket", None)
            if bucket is None:
                bucket = self.local.bucket = Bucket(self.stream_rate)
            bucket.acquire(n, self.cancel)


# ---------------------------------------------------------------- URL providers

def _private_file(path, what):
    """Read a small secret file: regular, not a symlink, ours, mode 0600-ish."""
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    except OSError:
        raise ProviderError(what) from None
    with os.fdopen(fd, "rb") as f:
        st = os.fstat(f.fileno())
        if not stat.S_ISREG(st.st_mode) or st.st_uid != os.geteuid() or st.st_mode & 0o077 or st.st_size > 1 << 20:
            raise ProviderError(what + " permissions")
        try:
            return json.loads(f.read().decode("utf-8"), object_pairs_hook=_pairs, parse_constant=_const)
        except (ValueError, UnicodeError, ManifestError):
            raise ProviderError(what + " format") from None


class UrlFileProvider:
    """signed source -> URL from a presigned-URL JSON file; re-read on every call so a refresher can replace it."""

    def __init__(self, path):
        self.path = path

    def __call__(self, ref):
        doc = _private_file(self.path, "urls file")
        if not isinstance(doc, dict) or doc.get("schema") != URLS_SCHEMA or not isinstance(doc.get("urls"), dict):
            raise ProviderError("urls file format")
        url = doc["urls"].get(ref)
        if type(url) is not str:
            raise ProviderError("no url for ref")
        return url


def sigv4_presign(host, path, region, access_key, secret_key, token, now, expires, scheme="https"):
    """AWS Signature V4 query presign of a GET (service s3, UNSIGNED-PAYLOAD). `path` is the already-encoded
    canonical URI. Returns the full URL; the caller must treat it as a secret."""
    day, stamp = now.strftime("%Y%m%d"), now.strftime("%Y%m%dT%H%M%SZ")
    scope = f"{day}/{region}/s3/aws4_request"
    query = {"X-Amz-Algorithm": "AWS4-HMAC-SHA256", "X-Amz-Credential": f"{access_key}/{scope}", "X-Amz-Date": stamp,
             "X-Amz-Expires": str(expires), "X-Amz-SignedHeaders": "host"}
    if token:
        query["X-Amz-Security-Token"] = token
    canon_query = "&".join(f"{urllib.parse.quote(k, safe='-_.~')}={urllib.parse.quote(v, safe='-_.~')}" for k, v in sorted(query.items()))
    canonical = f"GET\n{path}\n{canon_query}\nhost:{host}\n\nhost\nUNSIGNED-PAYLOAD"
    to_sign = f"AWS4-HMAC-SHA256\n{stamp}\n{scope}\n{hashlib.sha256(canonical.encode()).hexdigest()}"
    key = ("AWS4" + secret_key).encode()
    for part in (day, region, "s3", "aws4_request"):
        key = hmac.new(key, part.encode(), hashlib.sha256).digest()
    signature = hmac.new(key, to_sign.encode(), hashlib.sha256).hexdigest()
    return f"{scheme}://{host}{path}?{canon_query}&X-Amz-Signature={signature}"


@dataclasses.dataclass(frozen=True)
class S3Config:
    endpoint: str
    region: str
    access_key_id: str
    secret_access_key: str
    session_token: str = ""

    @staticmethod
    def load(environ, config_path=None):
        """Config file wins over the environment, field by field. Returns None when nothing usable is set."""
        doc = {}
        if config_path:
            doc = _private_file(config_path, "s3 config")
            if not isinstance(doc, dict) or set(doc) - {"endpoint", "region", "access_key_id", "secret_access_key", "session_token"}:
                raise ProviderError("s3 config format")
        pick = lambda key, env: doc.get(key) or environ.get(env, "")  # noqa: E731
        cfg = S3Config(pick("endpoint", "KF_S3_ENDPOINT"), pick("region", "KF_S3_REGION") or "auto",
                       pick("access_key_id", "AWS_ACCESS_KEY_ID"), pick("secret_access_key", "AWS_SECRET_ACCESS_KEY"),
                       pick("session_token", "AWS_SESSION_TOKEN"))
        if not (cfg.endpoint and cfg.access_key_id and cfg.secret_access_key):
            return None
        if not all(type(v) is str for v in dataclasses.astuple(cfg)) or not re.fullmatch(r"[a-z0-9-]{1,32}", cfg.region):
            raise ProviderError("s3 config format")
        return cfg


class S3Provider:
    """s3 source -> a locally signed presigned GET (path-style, 20 minutes). Credentials stay in this process."""

    def __init__(self, config, allow_http=False, clock=None):
        self.config = config
        self.allow_http = allow_http
        self.clock = clock or (lambda: datetime.datetime.now(datetime.timezone.utc))

    def __call__(self, bucket, key):
        c = self.config
        p = urllib.parse.urlsplit(c.endpoint)
        if p.scheme not in (("https", "http") if self.allow_http else ("https",)) or not p.netloc or p.path not in ("", "/") or p.query or p.username:
            raise ProviderError("s3 endpoint")
        path = "/" + urllib.parse.quote(bucket + "/" + key, safe="/-_.~")
        return sigv4_presign(p.netloc, path, c.region, c.access_key_id, c.secret_access_key, c.session_token, self.clock(), 1200, p.scheme)


# ---------------------------------------------------------------- the downloader

def hash_file(path, cancel=None):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while data := f.read(kf_fetch.BLOCK):
            if cancel is not None and cancel.is_set():
                return None
            h.update(data)
    return h.hexdigest()


def _is_link(path):
    try:
        return stat.S_ISLNK(os.lstat(path).st_mode)
    except FileNotFoundError:
        return False


def confined_dest(root, rel):
    """root/rel with every existing component checked: no symlink anywhere below root. Returns the Path."""
    cur = root
    for part in rel.split("/"):
        cur = cur / part
        if _is_link(cur):
            raise kf_fetch.FetchError("SIZE", local=True)
    return cur


def ensure_parent(root, rel):
    cur = root
    for part in rel.split("/")[:-1]:
        cur = cur / part
        if _is_link(cur):
            raise kf_fetch.FetchError("SIZE", local=True)
        try:
            os.mkdir(cur, 0o755)
        except FileExistsError:
            pass
        if _is_link(cur) or not cur.is_dir():
            raise kf_fetch.FetchError("SIZE", local=True)


def _name_of_file(value):
    return re.sub(r"[^A-Za-z0-9._-]", "_", value)[:100]


class Downloader:
    def __init__(self, manifest, root, *, url_provider=None, s3_provider=None, jobs=DEFAULT_JOBS, streams=DEFAULT_STREAMS,
                 chunk_size=16 * kf_fetch.BLOCK, limit_mbps=DEFAULT_LIMIT_MBPS, stream_limit_mbps=0.0, only=(), stages=(),
                 pins=None, pin_dirs=(), status_path=None, emit=None, allow_http=False, reserve_bytes=0,
                 disk_free=None, cancel_grace=10.0):
        self.manifest = manifest
        self.root = Path(root)
        self.url_provider, self.s3_provider = url_provider, s3_provider
        self.jobs, self.streams, self.chunk_size = jobs, streams, chunk_size
        self.pins = dict(pins or {})
        self.pin_dirs = [Path(p) for p in pin_dirs]
        self.allow_http = allow_http
        self.reserve_bytes = reserve_bytes
        self.disk_free = disk_free or (lambda p: shutil.disk_usage(p).free)
        self.cancel_grace = cancel_grace
        self.emit_line = emit or (lambda line: print(line, flush=True))
        self.state_dir = self.root / STATE_DIR
        self.status_path = Path(status_path) if status_path else self.state_dir / "status.json"
        self.cancel_event = threading.Event()
        self.throttle = Throttle(limit_mbps * 1e6, stream_limit_mbps * 1e6, self.cancel_event)
        self.queue = manifest.ordered(only, stages)
        self.cond = threading.Condition()
        self.results = {i.name: {"state": "pending", "size": i.size, "bytes": 0} for i in self.queue}
        self.active = {}
        self.started = time.time()
        self.run_state = "init"
        self.next_index = 0

    # output: only validated names and numbers, scrubbed once more
    def emit(self, line):
        self.emit_line(redact(line))

    def cancel(self):
        self.cancel_event.set()
        with self.cond:
            for dl in list(self.active.values()):
                dl.cancel()
            self.cond.notify_all()

    # status
    def write_status(self):
        with self.cond:
            items = {}
            for name, r in self.results.items():
                row = dict(r)
                dl = self.active.get(name)
                if dl is not None and row["state"] == "active":
                    row["bytes"] = dl.transferred
                items[name] = row
            doc = {"schema": STATUS_SCHEMA, "pid": os.getpid(), "state": self.run_state, "updated": round(time.time(), 3),
                   "started": round(self.started, 3), "root": str(self.root),
                   "dests": {i.name: i.dest for i in self.queue}, "items": items}
        tmp = self.status_path.with_name(self.status_path.name + ".tmp")
        try:
            with open(tmp, "w", encoding="utf-8") as f:
                json.dump(doc, f, sort_keys=True)
                f.write("\n")
            os.replace(tmp, self.status_path)
        except OSError:
            pass

    def finish_item(self, item, **fields):
        with self.cond:
            self.results[item.name].update(fields)
            self.cond.notify_all()
        self.write_status()

    def wait(self, names, deadline_s):
        """True when every name is verified; False when one failed or the deadline passed first."""
        end = time.monotonic() + deadline_s
        with self.cond:
            while True:
                states = [self.results[n]["state"] for n in names]
                if "failed" in states:
                    return False
                if all(s == "verified" for s in states):
                    return True
                left = end - time.monotonic()
                if left <= 0 or self.cancel_event.is_set():
                    return False
                self.cond.wait(min(left, 0.5))

    # one item
    def _url_for(self, source):
        if source.kind == "signed":
            if self.url_provider is None:
                raise ProviderError("no url provider")
            url = self.url_provider(source.ref)
        elif source.kind == "s3":
            if self.s3_provider is None:
                raise ProviderError("no s3 credentials")
            url = self.s3_provider(source.bucket, source.key)
        else:
            url = source.url
        try:
            return kf_fetch.valid_url(url, secure=not self.allow_http)
        except kf_fetch.FetchError:
            raise ProviderError("url") from None

    def _label(self, source, index):
        return {"signed": "r2", "s3": "s3"}.get(source.kind) or f"mirror{index}"

    def _local_candidates(self, item):
        paths = []
        if item.name in self.pins:
            paths.append(Path(self.pins[item.name]))
        if item.local:
            paths.append(Path(item.local))
        for d in self.pin_dirs:
            paths += [d / item.name, d / item.dest]
        return paths

    def _try_local(self, item, dest, unverified):
        for path in self._local_candidates(item):
            try:
                if not path.is_file() or path.stat().st_size != item.size:
                    continue
                if hash_file(path, self.cancel_event) != item.sha256:
                    continue
            except OSError:
                continue
            if self.cancel_event.is_set():
                return False
            if path.resolve() == dest.resolve():
                return True
            # copy through <dest>.unverified and verify the bytes that were actually written
            h, n = hashlib.sha256(), 0
            fd = os.open(unverified, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
            with os.fdopen(fd, "wb") as out, open(path, "rb") as src:
                while data := src.read(kf_fetch.BLOCK):
                    if self.cancel_event.is_set():
                        return False
                    h.update(data)
                    n += len(data)
                    out.write(data)
                out.flush()
                os.fsync(out.fileno())
            if n != item.size or h.hexdigest() != item.sha256:
                unverified.unlink(missing_ok=True)
                continue
            os.replace(unverified, dest)
            kf_fetch.fsync_directory(dest.parent)
            return "local"
        return False

    def _download(self, item, source, index, dest, state):
        """One source. Raises FetchError / ProviderError."""
        rounds = 3 if source.kind in ("signed", "s3") else 1
        for attempt in range(rounds):
            url = self._url_for(source)
            dl = kf_fetch.Download(url, dest, item.sha256, self.streams, self.chunk_size, state,
                                   source.follow_redirect, None, expected_size=item.size,
                                   emit=lambda line: None, throttle=self.throttle)
            with self.cond:
                if self.cancel_event.is_set():
                    raise kf_fetch.FetchError("NETWORK")
                self.active[item.name] = dl
            try:
                dl.run()
                return
            except kf_fetch.FetchError as e:
                # an expired presigned URL: ask the provider again (resume re-verifies every saved chunk)
                if e.kind == "AUTH" and attempt + 1 < rounds and not self.cancel_event.is_set():
                    continue
                raise
            finally:
                with self.cond:
                    self.active.pop(item.name, None)

    def fetch_item(self, item):
        t0 = time.monotonic()
        self.finish_item(item, state="active")
        state = self.state_dir / (hashlib.sha256(item.name.encode()).hexdigest()[:24] + ".json")
        cls = "SIZE"
        try:
            dest = confined_dest(self.root, item.dest)
            ensure_parent(self.root, item.dest)
            unverified = kf_fetch.unverified_path(dest)
            for derived in (unverified, state, Path(str(state) + ".lock"), Path(str(state) + ".tmp")):
                if _is_link(derived):
                    raise kf_fetch.FetchError("SIZE", local=True)
            source_used = None
            if dest.is_file() and dest.stat().st_size == item.size and hash_file(dest, self.cancel_event) == item.sha256:
                source_used = "present"
            elif self.cancel_event.is_set():
                raise kf_fetch.FetchError("NETWORK")
            else:
                got = self._try_local(item, dest, unverified)
                if got:
                    source_used = "local"
            if source_used is None:
                for index, source in enumerate(item.sources):
                    if self.cancel_event.is_set():
                        raise kf_fetch.FetchError("NETWORK")
                    try:
                        self._download(item, source, index, dest, state)
                        source_used = self._label(source, index)
                        break
                    except ProviderError:
                        cls = "AUTH"
                    except kf_fetch.FetchError as e:
                        if e.local:
                            raise
                        if self.cancel_event.is_set():
                            raise
                        cls = e.kind
                    # a fallback never mixes bytes from two sources
                    kf_fetch.discard_partial(dest, state)
            if source_used is None:
                raise kf_fetch.FetchError(cls)
            for extra in (state, Path(str(state) + ".lock"), Path(str(state) + ".tmp")):
                extra.unlink(missing_ok=True)
            secs = max(time.monotonic() - t0, 1e-6)
            rate = item.size / secs / 1e6
            self.finish_item(item, state="verified", bytes=item.size, secs=round(secs, 3), source=source_used, rate_mbps=round(rate, 3))
            self.emit(f"KF_STEP name=download.{item.name} secs={secs:.1f} bytes={item.size} source={source_used} MBps={rate:.2f}")
        except kf_fetch.FetchError as e:
            if self.cancel_event.is_set():
                self.finish_item(item, state="pending")
                return
            self.finish_item(item, state="failed", **{"class": "RETRY", "reason": e.kind})
            self.emit(f"KF_DOWNLOAD_FAIL name={item.name} class=RETRY reason={e.kind}")
        except OSError:
            self.finish_item(item, state="failed", **{"class": "RETRY", "reason": "DISK"})
            self.emit(f"KF_DOWNLOAD_FAIL name={item.name} class=RETRY reason=DISK")

    def claim(self):
        with self.cond:
            if self.next_index >= len(self.queue) or self.cancel_event.is_set():
                return None
            item = self.queue[self.next_index]
            self.next_index += 1
            return item

    def worker(self):
        while (item := self.claim()) is not None:
            try:
                self.fetch_item(item)
            except Exception:  # never a traceback with remote-controlled text
                self.finish_item(item, state="failed", **{"class": "RETRY", "reason": "INTERNAL"})
                self.emit(f"KF_DOWNLOAD_FAIL name={item.name} class=RETRY reason=INTERNAL")

    def preflight(self):
        need = self.reserve_bytes + sum(i.size for i in self.queue if not (self.root / i.dest).is_file())
        try:
            self.root.mkdir(parents=True, exist_ok=True)
            self.state_dir.mkdir(parents=True, exist_ok=True)
            free = self.disk_free(self.root)
        except OSError:
            return False
        return free >= need

    def run(self):
        """Blocks until every queued item is verified or failed, or cancel(). Returns the exit code."""
        if not self.preflight():
            self.emit("KF_DOWNLOAD_FAIL class=RETRY reason=DISK_RESERVE")
            return 2
        lock = open(self.state_dir / "downloader.lock", "a+b")
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            lock.close()
            self.emit("KF_DOWNLOAD_FAIL class=RETRY reason=ALREADY_RUNNING")
            return 2
        try:
            return self._run_locked()
        finally:
            lock.close()

    def _run_locked(self):
        self.run_state = "running"
        self.write_status()
        done = threading.Event()

        def ticker():
            while not done.wait(2.0):
                self.write_status()

        threading.Thread(target=ticker, daemon=True).start()
        threads = [threading.Thread(target=self.worker, daemon=True) for _ in range(max(1, min(self.jobs, len(self.queue) or 1)))]
        for t in threads:
            t.start()
        while any(t.is_alive() for t in threads):
            if self.cancel_event.is_set():
                end = time.monotonic() + self.cancel_grace
                for t in threads:
                    t.join(max(0.0, end - time.monotonic()))
                break
            for t in threads:
                t.join(0.2)
        done.set()
        failed = [n for n, r in self.results.items() if r["state"] == "failed"]
        if self.cancel_event.is_set():
            self.run_state, code = "cancelled", 5
        elif failed:
            self.run_state, code = "failed", 1
        else:
            self.run_state, code = "done", 0
        self.write_status()
        ok = sum(1 for r in self.results.values() if r["state"] == "verified")
        self.emit(f"KF_DOWNLOAD_DONE state={self.run_state} ok={ok} failed={len(failed)} total={len(self.results)}")
        return code


# ---------------------------------------------------------------- wait / status / verify

def read_status(path):
    try:
        with open(path, "rb") as f:
            raw = f.read((1 << 20) + 1)
        doc = json.loads(raw.decode("utf-8"))
    except (OSError, ValueError, UnicodeError):
        return None
    return doc if isinstance(doc, dict) and doc.get("schema") == STATUS_SCHEMA and isinstance(doc.get("items"), dict) else None


def _alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except OSError:
        return True


def wait_for(status_path, names, deadline_s, poll=0.25, out=print):
    """The shared code path of `wait ITEM...`: 0 all verified, 1 an item failed, 2 unknown item, 3 deadline,
    4 the downloader is gone before the items finished."""
    end = time.monotonic() + deadline_s
    while True:
        doc = read_status(status_path)
        if doc is not None:
            items = doc["items"]
            if any(n not in items for n in names):
                out("KF_WAIT unknown_item=1")
                return 2
            rows = [items[n] for n in names]
            if any(r.get("state") == "failed" for r in rows):
                out("KF_WAIT result=failed")
                return 1
            if all(r.get("state") == "verified" for r in rows):
                root = Path(str(doc.get("root", "")))
                for n in names:
                    rel = doc.get("dests", {}).get(n, "")
                    try:
                        st = os.lstat(root / rel) if valid_dest(rel) else None
                    except OSError:
                        st = None
                    if st is None or not stat.S_ISREG(st.st_mode) or st.st_size != items[n].get("size"):
                        out("KF_WAIT result=failed reason=missing_file")
                        return 1
                out("KF_WAIT result=verified items=" + str(len(names)))
                return 0
            pid = doc.get("pid")
            if doc.get("state") in ("done", "failed", "cancelled") or (type(pid) is int and not _alive(pid)):
                out("KF_WAIT result=downloader_gone")
                return 4
        if time.monotonic() >= end:
            out("KF_WAIT result=deadline")
            return 3
        time.sleep(poll)


def verify_items(manifest, root, only=(), stages=(), out=print):
    bad = 0
    for item in manifest.ordered(only, stages):
        try:
            path = confined_dest(Path(root), item.dest)
            ok = path.is_file() and path.stat().st_size == item.size and hash_file(path) == item.sha256
        except (OSError, kf_fetch.FetchError):
            ok = False
        out(f"KF_VERIFY name={item.name} ok={int(ok)}")
        bad += not ok
    return 1 if bad else 0


# ---------------------------------------------------------------- CLI

class Parser(argparse.ArgumentParser):
    def error(self, message):
        self.exit(2, "KF_DOWNLOAD_FAIL class=USAGE\n")


def _rate(text):
    value = float(text)
    if not 0 <= value <= 100000:
        raise ValueError(text)
    return value


def _dest_status(args):
    if args.status:
        return Path(args.status)
    return Path(args.dest or ".") / STATE_DIR / "status.json"


def build_parser():
    p = Parser(prog="box_downloader.py", description=__doc__.split("\n\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True, parser_class=Parser)
    f = sub.add_parser("fetch")
    f.add_argument("manifest")
    f.add_argument("--dest", default=".")
    f.add_argument("--only", nargs="+", default=[], metavar="NAME")
    f.add_argument("--stages", default="", help="colon-separated needed_by stages to fetch")
    f.add_argument("--jobs", type=int, default=DEFAULT_JOBS)
    f.add_argument("--streams", type=int, default=DEFAULT_STREAMS)
    f.add_argument("--chunk-mb", type=int, default=16)
    f.add_argument("--limit-bandwidth", type=_rate, default=DEFAULT_LIMIT_MBPS, metavar="MBPS", help="global cap, 0 = off")
    f.add_argument("--stream-limit", type=_rate, default=0.0, metavar="MBPS", help="per-connection cap, 0 = off")
    f.add_argument("--pin", action="append", default=[], metavar="NAME=PATH")
    f.add_argument("--pin-dir", action="append", default=[], metavar="DIR")
    f.add_argument("--urls", metavar="FILE", help="presigned-URL JSON file (kayfabe.download-urls/1)")
    f.add_argument("--s3-config", metavar="FILE")
    f.add_argument("--status", metavar="FILE")
    f.add_argument("--reserve-gb", type=float, default=0.0)
    f.add_argument("--no-nice", action="store_true")
    w = sub.add_parser("wait")
    w.add_argument("--dest")
    w.add_argument("--status")
    w.add_argument("--deadline", type=float, default=3600.0)
    w.add_argument("items", nargs="+", metavar="ITEM")
    s = sub.add_parser("status")
    s.add_argument("--dest")
    s.add_argument("--status")
    v = sub.add_parser("verify")
    v.add_argument("manifest")
    v.add_argument("--dest", default=".")
    v.add_argument("--only", nargs="+", default=[], metavar="NAME")
    c = sub.add_parser("check")
    c.add_argument("manifest")
    for sp in (f, v, c):
        sp.add_argument("--allow-http", action="store_true", help="accept http:// sources (loopback tests, trusted LANs)")
    return p


def cmd_fetch(args):
    manifest = load_manifest(args.manifest, args.allow_http)
    names = {i.name for i in manifest.items}
    stages = tuple(s for s in args.stages.split(":") if s)
    if any(n not in names for n in args.only) or any(s not in manifest.stages for s in stages):
        raise ManifestError("selection")
    pins = {}
    for spec in args.pin:
        name, sep, path = spec.partition("=")
        if not sep or name not in names or not path:
            raise ManifestError("pin")
        pins[name] = path
    if not 1 <= args.jobs <= 32 or not 1 <= args.streams <= 64 or not 1 <= args.chunk_mb <= 1024 or not 0 <= args.reserve_gb <= 1e6:
        raise ManifestError("limits")
    provider = UrlFileProvider(args.urls) if args.urls else None
    cfg = S3Config.load(os.environ, args.s3_config)
    s3 = S3Provider(cfg, args.allow_http) if cfg else None
    if not args.no_nice:
        try:
            os.nice(19)
        except OSError:
            pass
        try:
            import subprocess
            subprocess.run(["ionice", "-c3", "-p", str(os.getpid())], capture_output=True, timeout=5)
        except Exception:
            pass
    d = Downloader(manifest, Path(args.dest).resolve(), url_provider=provider, s3_provider=s3, jobs=args.jobs,
                   streams=args.streams, chunk_size=args.chunk_mb * kf_fetch.BLOCK, limit_mbps=args.limit_bandwidth,
                   stream_limit_mbps=args.stream_limit, only=tuple(args.only), stages=stages, pins=pins,
                   pin_dirs=args.pin_dir, status_path=args.status, allow_http=args.allow_http,
                   reserve_bytes=int(args.reserve_gb * 1e9))
    handler = lambda *_: d.cancel()  # noqa: E731
    signal.signal(signal.SIGTERM, handler)
    signal.signal(signal.SIGINT, handler)
    code = d.run()
    sys.stdout.flush()
    if code == 5:
        os._exit(5)  # a worker may still sit in a socket read; the partial files are already safe
    return code


def main(argv=None):
    args = build_parser().parse_args(argv)
    try:
        if args.cmd == "check":
            m = load_manifest(args.manifest, args.allow_http)
            print(f"KF_MANIFEST sha256={m.sha256} items={len(m.items)} bytes={sum(i.size for i in m.items)} stages={':'.join(m.stages)}")
            return 0
        if args.cmd == "fetch":
            return cmd_fetch(args)
        if args.cmd == "verify":
            m = load_manifest(args.manifest, args.allow_http)
            if any(n not in {i.name for i in m.items} for n in args.only):
                raise ManifestError("selection")
            return verify_items(m, args.dest, tuple(args.only))
        if args.cmd == "wait":
            return wait_for(_dest_status(args), args.items, args.deadline)
        doc = read_status(_dest_status(args))
        if doc is None:
            print("KF_STATUS state=none")
            return 4
        rows = doc["items"].values()
        print("KF_STATUS state=%s verified=%d failed=%d total=%d" % (
            str(doc.get("state"))[:20], sum(r.get("state") == "verified" for r in rows),
            sum(r.get("state") == "failed" for r in rows), len(doc["items"])))
        return 0
    except (ManifestError, ProviderError, OSError, ValueError):
        print("KF_DOWNLOAD_FAIL class=USAGE", flush=True)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
