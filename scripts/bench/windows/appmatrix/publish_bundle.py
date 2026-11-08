#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Immutable bundle publishing: stdlib S3 SigV4/multipart or caller-presigned URLs.

No credential files, external clients, redirects, arbitrary keys or log URLs.
Credentials/capability URLs stay in their matching protected read/upload process.
Build processes receive no credentials; only the optional public upload uses secrets.
Resume files contain upload IDs and verified part receipts, never credentials/URLs.
No network call occurs at import time. Tests use an in-memory store exclusively.
"""
import argparse
import base64
import datetime
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import package_bundle as B
import bundle_compliance as C

MIN_PART = 5 * 1024**2
DEFAULT_PART = 64 * 1024**2
MAX_PART = 5 * 1024**3


def public_base(env):
    base = env.get("BUNDLE_PUBLIC_BASE_URL", "")
    host = (urllib.parse.urlsplit(base).hostname or "").lower()
    if not C.https(base) or "?" in base or host.endswith((".r2.dev", ".r2.cloudflarestorage.com")):
        raise ValueError("a public bundles custom domain must be configured")
    return base.rstrip("/")


class PublishError(ValueError):
    """Safe diagnostic, never containing provider bodies, keys or signed URLs."""


class StoreError(PublishError):
    def __init__(self, status):
        self.status = status
        super().__init__(f"object service returned HTTP {status}")


def keys(digest, bundle_class="public"):
    if not re.fullmatch(r"[0-9a-f]{64}", digest): raise ValueError("invalid image digest")
    if bundle_class != "public": raise ValueError("only public release object keys are permitted")
    prefix = "winapps-open"
    return f"{prefix}/kfapps-{digest}.iso", f"{prefix}/manifest-{digest}.json"


def guard_publish(env, manual=False):
    if manual:
        if "CI" in env or "GITHUB_ACTIONS" in env:
            raise ValueError("manual publication cannot run in CI/Actions")
        if not env.get("BUNDLE_UPLOAD_URLS") or any(k in env for k in ("AWS_ACCESS_KEY_ID","AWS_SECRET_ACCESS_KEY","AWS_SESSION_TOKEN","R2_ACCESS_KEY_ID","R2_SECRET_ACCESS_KEY","KF_RUN_TOKEN")):
            raise ValueError("manual upload requires caller-presigned URLs only, without credentials/run tokens")
        return
    if (env.get("GITHUB_ACTIONS") != "true" or env.get("GITHUB_REPOSITORY") != "reindertpelsma/kayfabe"
            or env.get("GITHUB_REF") != "refs/heads/master"
            or env.get("GITHUB_EVENT_NAME") not in ("push", "workflow_dispatch")
            or env.get("BUNDLE_PUBLISH_APPROVED") != "true"):
        raise ValueError("publishing requires approved canonical master workflow context")


def part_ranges(size, part_size=DEFAULT_PART):
    if size <= 0 or not MIN_PART <= part_size <= MAX_PART:
        raise ValueError("invalid multipart size")
    count = (size + part_size - 1) // part_size
    if count > 10000: raise ValueError("multipart exceeds 10000 parts")
    return [(n + 1, n * part_size, min(part_size, size - n * part_size)) for n in range(count)]


def xml_root(body):
    try:
        root = ET.fromstring(body)
    except ET.ParseError as e:
        raise PublishError("invalid object-service XML") from e
    if root.tag.rsplit("}", 1)[-1] == "Error":
        raise PublishError("object service returned embedded XML error")
    return root


def xml_value(root, name):
    values = [x.text for x in root.iter() if x.tag.rsplit("}", 1)[-1] == name]
    if len(values) != 1 or not values[0]: raise PublishError("missing object-service field")
    return values[0]


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        raise PublishError("object-service redirects refused")


def send(method, url, headers, data):
    """All network exceptions are sanitized; signed URLs never reach diagnostics."""
    request = urllib.request.Request(url, data=data if method != "HEAD" else None, headers=headers, method=method)
    try:
        with urllib.request.build_opener(NoRedirect()).open(request, timeout=180) as response:
            body = response.read(2 * 1024**2 + 1)
            if len(body) > 2 * 1024**2: raise PublishError("object-service response exceeds limit")
            return {k.lower(): v for k, v in response.headers.items()}, body
    except urllib.error.HTTPError as e:
        raise StoreError(e.code) from None
    except (OSError, urllib.error.URLError):
        raise PublishError("object-service transport failed") from None


def sign_headers(method, url, headers, data, access_key, secret_key, stamp, region="auto", session_token=None):
    """AWS4-HMAC-SHA256, S3 canonical URI/query, signed payload and headers."""
    u = urllib.parse.urlsplit(url)
    headers = {k.lower(): " ".join(str(v).split()) for k, v in headers.items()}
    headers.update(host=u.netloc, **{"x-amz-date": stamp, "x-amz-content-sha256": hashlib.sha256(data).hexdigest()})
    if session_token: headers["x-amz-security-token"] = session_token
    # Our URLs are built from unescaped bucket/key strings once. Query values are
    # re-encoded independently and sorted after encoding, as required by SigV4.
    uri = urllib.parse.quote(urllib.parse.unquote(u.path), safe="/~")
    query = "&".join(k + "=" + v for k, v in sorted((urllib.parse.quote(k, safe="~"), urllib.parse.quote(v, safe="~"))
                       for k, v in urllib.parse.parse_qsl(u.query, keep_blank_values=True)))
    names = ";".join(sorted(headers))
    canonical = "\n".join((method, uri, query, "".join(k + ":" + headers[k] + "\n" for k in sorted(headers)), names, headers["x-amz-content-sha256"]))
    scope = stamp[:8] + "/" + region + "/s3/aws4_request"
    message = "AWS4-HMAC-SHA256\n" + stamp + "\n" + scope + "\n" + hashlib.sha256(canonical.encode()).hexdigest()
    signing = ("AWS4" + secret_key).encode()
    for value in (stamp[:8], region, "s3", "aws4_request"):
        signing = hmac.new(signing, value.encode(), hashlib.sha256).digest()
    signature = hmac.new(signing, message.encode(), hashlib.sha256).hexdigest()
    headers["authorization"] = f"AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={names}, Signature={signature}"
    return headers


class S3Store:
    def __init__(self, endpoint, bucket, access_key, secret_key, session_token=None, sender=send):
        u = urllib.parse.urlsplit(endpoint)
        if u.scheme != "https" or not u.hostname or u.username or u.password or u.path not in ("", "/") or u.query or u.fragment:
            raise ValueError("S3 endpoint must be an HTTPS origin")
        if not re.fullmatch(r"[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]", bucket): raise ValueError("invalid dedicated bucket")
        if not access_key or not secret_key: raise ValueError("missing environment upload credentials")
        self.endpoint, self.bucket = endpoint.rstrip("/"), bucket
        self.access_key, self.secret_key, self.session_token = access_key, secret_key, session_token
        self.sender = sender

    def request(self, method, key, query=None, headers=None, data=b""):
        url = self.endpoint + "/" + self.bucket + "/" + urllib.parse.quote(key, safe="/~")
        if query is not None: url += "?" + urllib.parse.urlencode(sorted(query.items()))
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        signed = sign_headers(method, url, headers or {}, data, self.access_key, self.secret_key, stamp, session_token=self.session_token)
        return self.sender(method, url, signed, data)


class PresignedStore:
    """Caller initiates multipart and supplies per-method URLs in an env JSON map.

    Each key maps to {head, get?, put?} or {head, upload_id, part_size, parts:
    {"1": url,...}, complete, abort, list}. No capability URL is persisted.
    Conditional final URLs must bind if-none-match in X-Amz-SignedHeaders.
    """
    def __init__(self, plans, sender=send):
        self.plans, self.sender = plans, sender

    def request(self, method, key, query=None, headers=None, data=b""):
        p = self.plans[key]; query = query or {}; headers = headers or {}
        if method == "POST" and "uploads" in query:
            root = ET.Element("InitiateMultipartUploadResult"); ET.SubElement(root, "UploadId").text = p["upload_id"]
            return {}, ET.tostring(root)
        if "uploadId" in query:
            if query["uploadId"] != p["upload_id"]: raise PublishError("presigned upload ID mismatch")
            if method == "PUT": url = p["parts"][str(query["partNumber"])]
            else: url = p[{"POST": "complete", "DELETE": "abort", "GET": "list"}[method]]
        else: url = p[method.lower()]
        u = urllib.parse.urlsplit(url)
        if u.scheme != "https" or not u.hostname or u.username or u.password or u.fragment or not urllib.parse.unquote(u.path).endswith("/" + key):
            raise PublishError("invalid presigned object capability")
        q = dict(urllib.parse.parse_qsl(u.query, keep_blank_values=True))
        if "uploadId" in query and q.get("uploadId") != query["uploadId"]:
            raise PublishError("presigned URL upload ID mismatch")
        if "partNumber" in query and q.get("partNumber") != str(query["partNumber"]):
            raise PublishError("presigned URL part number mismatch")
        if "If-None-Match" in headers and "if-none-match" not in q.get("X-Amz-SignedHeaders", "").lower().split(";"):
            raise PublishError("presigned URL does not bind the create-only condition")
        return self.sender(method, url, headers, data)


def existing(store, key):
    try:
        return store.request("HEAD", key)[0]
    except StoreError as e:
        if e.status == 404: return None
        raise


def metadata(digest, inputs):
    return {"x-amz-meta-image-sha256": digest, "x-amz-meta-inputs": inputs}


def matches(headers, digest, inputs, size=None):
    return (headers.get("x-amz-meta-image-sha256") == digest and headers.get("x-amz-meta-inputs") == inputs
            and (size is None or headers.get("content-length") == str(size)))


def save_state(path, state):
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_bytes(B.canonical(state)); os.replace(tmp, path)


def multipart(store, path, key, digest, inputs, state_path, part_size=DEFAULT_PART):
    path, state_path = Path(path), Path(state_path)
    if part_size > DEFAULT_PART: raise ValueError("client multipart buffer limit is 64 MiB")
    size = path.stat().st_size
    ranges = part_ranges(size, part_size)
    head = existing(store, key)
    if head is not None:
        if not matches(head, digest, inputs, size): raise PublishError("existing ISO key conflicts with expected identity")
        return
    binding = dict(key=key, sha256=digest, inputs=inputs, size=size, part_size=part_size)
    if state_path.exists():
        state = B.load(state_path)
        if any(state.get(k) != v for k, v in binding.items()): raise PublishError("resume state does not match artifact")
    else:
        _, body = store.request("POST", key, {"uploads": ""}, dict(metadata(digest, inputs), **{"Content-Type": "application/octet-stream"}))
        state = dict(binding, upload_id=xml_value(xml_root(body), "UploadId"))
        save_state(state_path, state)
    upload_id = state["upload_id"]
    # List remote parts on every invocation; local completed-part entries are not
    # trusted. Pagination is bounded by S3's 10000-part limit.
    remote, marker = {}, 0
    while True:
        _, body = store.request("GET", key, {"uploadId": upload_id, "part-number-marker": str(marker)})
        root = xml_root(body)
        for part in root.iter():
            if part.tag.rsplit("}", 1)[-1] == "Part":
                n = int(xml_value(part, "PartNumber")); remote[n] = (xml_value(part, "ETag"), int(xml_value(part, "Size")))
        truncated = [x.text for x in root.iter() if x.tag.rsplit("}", 1)[-1] == "IsTruncated"]
        if truncated != ["true"]: break
        next_marker = int(xml_value(root, "NextPartNumberMarker"))
        if not marker < next_marker <= 10000: raise PublishError("invalid multipart pagination")
        marker = next_marker
    complete = ET.Element("CompleteMultipartUpload")
    whole = hashlib.sha256()
    known = state.get("parts", {})
    with path.open("rb", buffering=0) as source:
        for number, offset, length in ranges:
            source.seek(offset); data = source.read(length)
            if len(data) != length: raise PublishError("artifact changed length during upload")
            whole.update(data)
            part_sha = hashlib.sha256(data).hexdigest()
            saved = known.get(str(number), {})
            if (saved.get("sha256") == part_sha and number in remote
                    and remote[number] == (saved.get("etag"), length)):
                etag = saved["etag"]
            else:
                for attempt in range(3):
                    try:
                        headers, _ = store.request("PUT", key, {"uploadId": upload_id, "partNumber": str(number)},
                            {"Content-MD5": base64.b64encode(hashlib.md5(data).digest()).decode("ascii")}, data)
                        etag = headers.get("etag")
                        if not etag: raise PublishError("upload part returned no ETag")
                        break
                    except (PublishError, StoreError):
                        if attempt == 2: raise
                        time.sleep(2 ** attempt)
                known[str(number)] = dict(etag=etag, sha256=part_sha)
                state["parts"] = known; save_state(state_path, state)
            part = ET.SubElement(complete, "Part")
            ET.SubElement(part, "PartNumber").text = str(number); ET.SubElement(part, "ETag").text = etag
    if whole.hexdigest() != digest: raise PublishError("artifact SHA changed before multipart completion")
    try:
        _, body = store.request("POST", key, {"uploadId": upload_id},
            {"If-None-Match": "*", "Content-Type": "application/xml"}, ET.tostring(complete))
        if xml_root(body).tag.rsplit("}", 1)[-1] != "CompleteMultipartUploadResult":
            raise PublishError("unexpected multipart completion response")
    except StoreError as e:
        if e.status not in (409, 412): raise
        # A concurrent publisher won. Abort only our own pending multipart data;
        # never delete or rewrite the existing object.
        try:
            store.request("DELETE", key, {"uploadId": upload_id})
        except StoreError as abort_error:
            if abort_error.status != 404: raise
        state_path.unlink(missing_ok=True)
        head = existing(store, key)
        if head is None or not matches(head, digest, inputs, size): raise PublishError("conditional multipart conflict")
    head = existing(store, key)
    if head is None or not matches(head, digest, inputs, size): raise PublishError("completed ISO metadata mismatch")
    state_path.unlink(missing_ok=True)


def publish(store, iso, receipt, state_path, part_size=DEFAULT_PART, bundle_class="public"):
    require_class(receipt, bundle_class)
    digest, inputs = receipt["image"]["sha256"], receipt["inputs"]
    if not re.fullmatch(r"[0-9a-f]{64}", inputs): raise ValueError("invalid input digest")
    iso_key, manifest_key = keys(digest, bundle_class)
    if Path(iso).stat().st_size != receipt["image"]["size"] or B.sha256(iso) != digest:
        raise PublishError("local ISO does not match receipt")
    publish_sources(store, iso, receipt, state_path, part_size, bundle_class)
    multipart(store, iso, iso_key, digest, inputs, state_path, part_size)
    head = existing(store, manifest_key)
    if head is None:
        try:
            store.request("PUT", manifest_key, headers=dict(metadata(digest, inputs), **{"If-None-Match": "*", "Content-Type": "application/json"}), data=B.canonical(receipt))
        except StoreError as e:
            if e.status != 412: raise
    # Reuse the first receipt after a retry: packaging dates may differ, but its
    # actual bytes must verify the exact ISO and input identity we just built.
    _, body = store.request("GET", manifest_key)
    try: stored = json.loads(body)
    except (ValueError, UnicodeError): raise PublishError("stored receipt is invalid") from None
    if stored.get("inputs") != inputs or stored.get("image", {}).get("sha256") != digest:
        raise PublishError("existing receipt key conflicts with expected identity")
    B.verify_image(iso, stored)
    return iso_key, manifest_key


def publish_sources(store, iso, receipt, state_path, part_size, bundle_class="public"):
    """Mirror exact sources BEFORE making object code available. Never overwrite.

    Sources are streamed from the verified ISO into one ephemeral local file at
    a time. Identical source hashes are shared across package/build versions;
    their upload identity is the source hash, independent of an ISO's input hash.
    Only resume IDs/hashes are persisted. The existing multipart code handles
    arbitrarily large source archives with the same bounded memory as the ISO.
    """
    require_class(receipt, bundle_class)
    expected = [dict(package=p["id"], path=f"SOURCE/{p['id']}/{s['name']}", key=C.source_key(s, bundle_class), **s)
                for p in sorted(receipt["packages"], key=lambda p: p["id"]) for s in p["compliance"]["sources"]]
    if receipt.get("sources") != expected: raise PublishError("source mirror coverage mismatch")
    done = set()
    with B.bundle_iso.Reader(iso) as image, tempfile.TemporaryDirectory(prefix="source-mirror-", dir=Path(iso).parent) as tmp:
        for source in expected:
            key = source["key"]
            if key in done: continue
            path = Path(tmp) / "source.archive"
            with path.open("wb") as out:
                for block in image.chunks(source["path"]): out.write(block)
            if path.stat().st_size != source["size"] or B.sha256(path) != source["sha256"]:
                raise PublishError("source archive does not match pin")
            state = Path(state_path).with_name("source-" + source["sha256"] + ".multipart.json")
            multipart(store, path, key, source["sha256"], source["sha256"], state, part_size)
            path.unlink(); done.add(key)


def require_public(receipt):
    if receipt.get("bundle_class") != "public" or any(p["id"] not in B.PUBLIC_PACKAGES or p["compliance"]["distribution"] != "public-ok" for p in receipt["packages"]):
        raise PublishError("private media is forbidden in the public publisher")


def require_class(receipt, bundle_class):
    if bundle_class != "public": raise PublishError("only public release publication is permitted")
    require_public(receipt)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--state", type=Path)
    ap.add_argument("--manual-presigned", action="store_true", help="coordinator's LOCAL public-only upload using caller-presigned URLs; never allowed in CI")
    ap.add_argument("--class", dest="bundle_class", choices=("public", "private"), default="public")
    a = ap.parse_args(argv)
    try:
        guard_publish(os.environ, manual=a.manual_presigned)
        if a.bundle_class != "public": raise PublishError("private publication retired")
        if a.bundle_class == "public": public_base(os.environ)
        basename = "winapps-open"
        receipt = B.load(a.out / (basename + ".manifest.json"))
        require_class(receipt, a.bundle_class)
        expected = B.input_hash(B.load(HERE / "manifest.json"), B.load(HERE / "gate_profiles.json"), bundle_class=a.bundle_class)
        if receipt.get("inputs") != expected: raise PublishError("receipt does not match the reviewed checkout inputs")
        B.select_packages(receipt, B.load(HERE / "gate_profiles.json"), bundle_class=a.bundle_class)
        B.verify_image(a.out / (basename + ".iso"), receipt)
        if os.environ.get("BUNDLE_UPLOAD_URLS"):
            plans = json.loads(os.environ["BUNDLE_UPLOAD_URLS"])
            allowed_keys = set(keys(receipt["image"]["sha256"], a.bundle_class)) | {s["key"] for s in receipt["sources"]}
            if set(plans) != allowed_keys: raise PublishError("presigned plan key mismatch")
            store = PresignedStore(plans)
            part_size = plans[keys(receipt["image"]["sha256"], a.bundle_class)[0]]["part_size"]
        else:
            store = S3Store(os.environ["R2_ENDPOINT"], os.environ["R2_BUCKET"], os.environ["AWS_ACCESS_KEY_ID"], os.environ["AWS_SECRET_ACCESS_KEY"], os.environ.get("AWS_SESSION_TOKEN"))
            part_size = DEFAULT_PART
        publish(store, a.out / (basename + ".iso"), receipt, a.state or a.out / "multipart-state.json", part_size, bundle_class=a.bundle_class)
        (a.out / "published.pin.line").write_bytes(B.pin_text(receipt["image"]["sha256"], expected, a.bundle_class, B.bundle_repro.inventory_hash(receipt)).encode("ascii"))
        print("PUBLISH_OK sha256=" + receipt["image"]["sha256"])
        return 0
    except (OSError, ValueError, KeyError, TypeError):
        # Never format an exception from a network/credential/capability layer.
        print("PUBLISH_FAIL (no credential, URL or provider body logged)", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
