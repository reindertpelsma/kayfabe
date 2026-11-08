#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Regenerate the NVIDIA public installer catalog using only Python's stdlib.

Default: public discovery + HEADs + Linux checksum sidecars, never installers.
--offline: rebuild from normalized observations without any network.
--hash-windows VERSION: explicitly hash at most three missing desktop pins,
serially in /tmp, deleting each file before the next. No hardware or login.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import datetime as dt
import hashlib
import html
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import shutil
import sys
import tempfile
import threading
import urllib.error
import urllib.parse
import urllib.request

HERE = Path(__file__).resolve().parent
LINUX_INDEX = "https://download.nvidia.com/XFree86/Linux-x86_64/"
LINUX_ARCHIVE = "https://www.nvidia.com/en-us/drivers/unix/linux-display-archive/"
RELEASES = "https://docs.nvidia.com/datacenter/tesla/drivers/releases.json"
WINDOWS_HISTORY = "https://www.nvidia.com/en-us/drivers/rtx-enterprise-and-quadro-driver-branch-history/"
API = "https://gfwsl.geforce.com/services_toolkit/services/com/nvidia/services/AjaxDriverService.php"
GIB = 1024 ** 3
VERSION = r"[0-9]{3}\.[0-9]{1,3}(?:\.[0-9]{1,3})?"


def now():
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def version_key(value):
    if not re.fullmatch(VERSION, value):
        raise ValueError("invalid exact version")
    parts = list(map(int, value.split(".")))
    return tuple((parts + [0])[:3])


def official_url(raw):
    u = urllib.parse.urlsplit(raw)
    host = u.hostname or ""
    return (u.scheme == "https" and not u.username and not u.password and not u.port
            and not u.fragment and (host == "nvidia.com" or host.endswith(".nvidia.com")
                                   or host == "gfwsl.geforce.com"))


class OfficialRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if not official_url(newurl):
            raise ValueError("unofficial redirect refused")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


class Client:
    """Small public-text cache outside git. HEAD never falls back to GET."""
    def __init__(self, cache=None, timeout=20):
        self.cache = cache
        self.timeout = timeout
        self.receipts = {}
        self.lock = threading.Lock()
        self.opener = urllib.request.build_opener(OfficialRedirect())
        if cache:
            cache.mkdir(parents=True, exist_ok=True)

    def request(self, url, method="GET"):
        if not official_url(url):
            raise ValueError("unofficial URL refused")
        # Only hash_download is allowed to GET an installer.
        if method == "GET" and urllib.parse.urlsplit(url).path.endswith((".run", ".exe")):
            raise ValueError("installer GET requires explicit bounded hash mode")
        cache_key = hashlib.sha256((method + " " + url).encode()).hexdigest()
        path = self.cache / (cache_key + ".json") if self.cache else None
        if path and path.exists():
            row = json.loads(path.read_text())
        else:
            row = {"status": None, "body": "", "headers": {}, "final_url": url,
                   "checked_at": now(), "error": None}
            request = urllib.request.Request(url, method=method, headers={"User-Agent": "kayfabe-DRV1-public-catalog/1"})
            try:
                with self.opener.open(request, timeout=self.timeout) as reply:
                    row.update(status=reply.status, final_url=reply.url,
                               headers={k.lower(): v for k, v in reply.headers.items()})
                    if method == "GET":
                        raw = reply.read(8 * 1024 ** 2 + 1)
                        if len(raw) > 8 * 1024 ** 2:
                            raise ValueError("public metadata exceeds 8 MiB")
                        row["body"] = raw.decode("utf-8")
            except urllib.error.HTTPError as e:
                row.update(status=e.code, final_url=e.url, error="HTTP_ERROR")
            except (urllib.error.URLError, TimeoutError, OSError) as e:
                row["error"] = type(e).__name__
            if path:
                path.write_text(json.dumps(row, sort_keys=True) + "\n")
        receipt = {"url": url, "method": method, "status": row["status"], "checked_at": row["checked_at"],
                   "body_sha256": hashlib.sha256(row["body"].encode()).hexdigest() if method == "GET" and row["status"] == 200 else None}
        with self.lock:
            self.receipts[method + " " + url] = receipt
        return row

    def text(self, url):
        row = self.request(url)
        if row["status"] != 200:
            raise ValueError(f"required discovery source HTTP {row['status']}: {url}")
        return row["body"]


def versions_from_index(text):
    versions = set(re.findall(r"href=['\"](" + VERSION + r")/['\"]", text))
    return sorted((v for v in versions if version_key(v)[0] >= 535), key=version_key)


def linux_urls(version, releases, historical):
    urls = [f"{LINUX_INDEX}{version}/NVIDIA-Linux-x86_64-{version}.run"]
    for data in releases.values():
        for info in data["driver_info"]:
            if info["release_version"] == version:
                u = info.get("runfile_url", {}).get("x86_64")
                if u:
                    urls.append(u)
    if version in historical:
        urls.append(historical[version])
    urls.append(f"https://download.nvidia.com/tesla/{version}/NVIDIA-Linux-x86_64-{version}.run")
    return list(dict.fromkeys(urls))


def plain(text):
    return html.unescape(re.sub(r"<[^>]*>", " ", text)).strip()


def parse_date(value):
    value = value.strip().replace("April", "Apr").replace("July", "Jul").replace("June", "Jun")
    for fmt in ("%Y-%m-%d", "%B %d, %Y", "%b %d, %Y", "%a %b %d, %Y", "%d-%b-%y"):
        try:
            return dt.datetime.strptime(value, fmt).date().isoformat()
        except ValueError:
            pass
    return None


def parse_linux_archive(text):
    rows = []
    for part in re.split(r"<h[34]\b", text, flags=re.I)[1:]:
        text_part = plain(part)
        v = re.search(r"Version:\s*(" + VERSION + r")", text_part)
        d = re.search(r"Release Date:\s*([A-Za-z]+\s+\d+,\s*\d{4})", text_part)
        if v and version_key(v[1])[0] >= 535 and "Linux 64-bit" in text_part:
            rows.append({"version": v[1], "release_date": parse_date(d[1]) if d else None})
    return rows


class Tables(HTMLParser):
    def __init__(self):
        super().__init__()
        self.rows, self.row, self.cell = [], None, None

    def handle_starttag(self, tag, attrs):
        if tag == "tr":
            self.row = []
        elif tag == "td" and self.row is not None:
            self.cell = {"text": "", "href": None}
        elif tag == "a" and self.cell is not None:
            self.cell["href"] = dict(attrs).get("href")

    def handle_data(self, data):
        if self.cell is not None:
            self.cell["text"] += data

    def handle_endtag(self, tag):
        if tag == "td" and self.cell is not None:
            self.row.append(self.cell)
            self.cell = None
        elif tag == "tr" and self.row is not None:
            self.rows.append(self.row)
            self.row = None


def parse_windows_history(text):
    parser = Tables()
    parser.feed(text)
    rows = []
    for cells in parser.rows:
        values = [c["text"].strip() for c in cells]
        if len(values) == 4 and re.fullmatch(r"R\d{3}", values[0]) and re.fullmatch(VERSION, values[2]):
            branch = int(values[0][1:])
            if branch >= 535:
                rows.append({"branch": branch, "version": values[2], "release_date": parse_date(values[3]),
                             "details_url": cells[2]["href"]})
    return rows


def parse_driver_api(data):
    if int(data.get("Success", 0)) < 1:
        raise ValueError("no driver results")
    rows = []
    for item in data.get("IDS", []):
        d = item.get("downloadInfo", {})
        url, v = html.unescape(d.get("DownloadURL", "")), d.get("Version", "")
        if not re.fullmatch(VERSION, v) or version_key(v)[0] < 535:
            continue
        path = urllib.parse.urlsplit(url).path.lower()
        if not official_url(url) or d.get("IsWHQL") != "1" or "-desktop-" not in path or not path.endswith("whql.exe"):
            continue
        release = d.get("Release", "")
        rows.append({"os": "windows", "version": v, "branch": int(release) if release.isdigit() else None,
                     "branch_evidence": {"branch": int(release) if release.isdigit() else None, "version": v, "source_url": None},
                     "url": url, "release_date": parse_date(d.get("ReleaseDateTime", d.get("ReleaseDate", ""))),
                     "product_line": "rtx-enterprise" if "quadro" in path else "geforce",
                     "details_url": d.get("DetailsURL"), "discovery_source_urls": []})
    return rows


def windows_search_url(branch, studio):
    # Flags are NVIDIA's current driver-search JavaScript's Studio/Game Ready
    # contract. Independently require IsWHQL=1 on each admitted result.
    query = {"func": "DriverManualLookup", "psid": 107, "pfid": 877, "osID": 57, "languageCode": 1033,
             "beta": 0, "isWHQL": 0 if studio else 1, "dltype": -1, "dch": 1, "sort1": 1,
             "numberOfResults": 20, "release": branch, "upCRD": int(studio)}
    return API + "?" + urllib.parse.urlencode(query)


def parse_checksum(body, filename):
    found = set()
    for line in body.splitlines():
        m = re.fullmatch(r"([0-9a-fA-F]{64})\s+\*?(.+?)\s*", line)
        bsd = re.fullmatch(r"SHA256 \((.+)\) = ([0-9a-fA-F]{64})\s*", line)
        if m and m[2] in (filename, "./" + filename):
            found.add(m[1].lower())
        if bsd and bsd[1] == filename:
            found.add(bsd[2].lower())
    if len(found) > 1:
        raise ValueError("conflicting sha256 values for exact installer")
    return next(iter(found), None)


def collect_installer(candidate, client, previous=None):
    d = dict(candidate)
    d.setdefault("release_date", None)
    d.setdefault("last_tested", None)
    d.setdefault("test_records", [])
    d.update(sha256=None, size=None, checksum_source=None)
    url = d["url"]
    head = client.request(url, "HEAD")
    d["probe"] = {"method": "HEAD", "status": head["status"], "checked_at": head["checked_at"],
                  "final_url": head.get("final_url", url), "error": head.get("error")}
    if head["status"] != 200:
        d["status"] = "UNAVAILABLE" if head["status"] == 404 else "UNKNOWN:probe-error"
        return d
    length = head["headers"].get("content-length", "")
    if not re.fullmatch(r"[0-9]+", length) or int(length) <= 0:
        d["status"] = "UNKNOWN:probe-error"
        d["probe"]["error"] = "MISSING_CONTENT_LENGTH"
        return d
    d["size"] = int(length)
    d["status"] = "UNPINNED:needs-download" if d["os"] == "windows" else "UNPINNED:no-sha256"
    if d["os"] == "linux":
        directory, filename = url.rsplit("/", 1)
        for checksum_url in (url + ".sha256sum", directory + "/sha256sum.txt", directory + "/SHA256SUMS"):
            reply = client.request(checksum_url)
            sha = parse_checksum(reply["body"], filename) if reply["status"] == 200 else None
            if sha:
                d.update(status="PINNED", sha256=sha, checksum_source={"kind": "nvidia-sha256", "url": checksum_url, "checked_at": reply["checked_at"]})
                break
    elif previous and previous.get("status") == "PINNED" and previous.get("url") == url and previous.get("size") == d["size"]:
        # Preserve an earlier streamed measurement, including its original date.
        d.update(status="PINNED", sha256=previous["sha256"], checksum_source=previous["checksum_source"])
    return d


def hash_stream(stream, expected_size, directory):
    if not 0 < expected_size <= 15 * GIB:
        raise ValueError("download exceeds 15 GiB guard")
    if shutil.disk_usage("/")[2] - expected_size < 6 * GIB:
        raise ValueError("download would leave less than 6 GiB free")
    # TemporaryDirectory/finally removes bytes on success, truncation, exception
    # or KeyboardInterrupt; the caller hashes sequentially, never in its pool.
    with tempfile.TemporaryDirectory(prefix="DRV1-nvidia-", dir=directory) as tmp:
        digest, size = hashlib.sha256(), 0
        with (Path(tmp) / "installer.bin").open("xb") as out:
            while chunk := stream.read(1024 ** 2):
                size += len(chunk)
                if size > expected_size:
                    raise ValueError("download size exceeds HEAD")
                out.write(chunk)
                digest.update(chunk)
        if size != expected_size:
            raise ValueError("download size differs from HEAD")
        return digest.hexdigest()


def hash_download(d, client):
    if d["os"] != "windows" or d["status"] != "UNPINNED:needs-download":
        raise ValueError("only missing available Windows pins can be downloaded")
    # Serialize even concurrent invocations of the explicit hash option.
    import fcntl
    with open("/tmp/DRV1-driver-hash.lock", "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if shutil.disk_usage("/")[2] - d["size"] < 6 * GIB or d["size"] > 15 * GIB:
            raise ValueError("download disk/15 GiB guard")
        request = urllib.request.Request(d["url"], headers={"User-Agent": "kayfabe-DRV1-public-catalog/1"})
        with client.opener.open(request, timeout=client.timeout) as stream:
            if stream.status != 200 or not official_url(stream.url):
                raise ValueError("download requires official HTTP 200")
            sha = hash_stream(stream, d["size"], Path("/tmp"))
        d.update(status="PINNED", sha256=sha, checksum_source={"kind": "download-sha256", "url": d["url"], "checked_at": now()})


def discover(client, curated):
    linux = versions_from_index(client.text(LINUX_INDEX))
    releases = json.loads(client.text(RELEASES))
    archive = {r["version"]: r for r in parse_linux_archive(client.text(LINUX_ARCHIVE))}
    history = parse_windows_history(client.text(WINDOWS_HISTORY))
    if not linux or not history:
        raise ValueError("empty essential discovery source")
    branch_meta, candidates, gaps, exclusions = {}, {}, [], []

    def add(row):
        key = (row["os"], version_key(row["version"]))
        if key not in candidates:
            candidates[key] = row
        else:
            existing = candidates[key]
            if row.get("product_line") == "geforce" and existing.get("product_line") == "rtx-enterprise":
                row["candidate_urls"] = [row["url"], existing["url"]]
                row["discovery_source_urls"] = sorted(set(row["discovery_source_urls"] + existing["discovery_source_urls"]))
                candidates[key] = row
                return
            existing["discovery_source_urls"] = sorted(set(existing["discovery_source_urls"] + row["discovery_source_urls"]))
            for url in row.get("candidate_urls", [row["url"]]):
                if url not in existing.setdefault("candidate_urls", [existing["url"]]):
                    existing["candidate_urls"].append(url)
            if row.get("release_date"):
                existing["release_date"] = row["release_date"]
            if row.get("branch"):
                existing["branch"] = row["branch"]

    date_by_version = {v["release_version"]: v["release_date"] for branch in releases.values() for v in branch["driver_info"]}
    for key, data in releases.items():
        if int(key) >= 535:
            branch_meta[int(key)] = {"id": int(key), "branch_type": {"lts branch": "LTS", "production branch": "production", "new feature branch": "new-feature"}.get(data["type"], "UNKNOWN"), "source_url": RELEASES, "type_scope": "NVIDIA data center designation"}
            for info in data["driver_info"]:
                if isinstance(info.get("runfile_url"), dict) and info["runfile_url"].get("x86_64"):
                    linux.append(info["release_version"])
    for ver in sorted(set(linux + curated["historical_linux_candidates"]), key=version_key):
        branch = version_key(ver)[0]
        if branch < 535:
            continue
        urls = linux_urls(ver, releases, curated.get("historical_linux_urls", {}))
        src = [LINUX_INDEX] if ver in linux else [curated["historical_candidates_source"]["official_probe_root"]]
        if ver in date_by_version:
            src.append(RELEASES)
        add({"os": "linux", "version": ver, "branch": branch, "url": urls[0], "candidate_urls": urls,
             "release_date": date_by_version.get(ver, archive.get(ver, {}).get("release_date")), "discovery_source_urls": src,
             "discovery_kind": "official-listing" if ver in linux else "historical-tag-candidate"})
        branch_meta.setdefault(branch, {"id": branch, "branch_type": "UNKNOWN", "source_url": LINUX_INDEX, "type_scope": "UNKNOWN"})
    for key, row in curated.get("branch_type_overrides", {}).items():
        branch_meta[int(key)] = {"id": int(key), **row}

    # Enterprise history supplies authoritative R535 -> 539.x etc mapping. Get
    # details for these rows; only desktop WHQL URLs survive the parser.
    def history_details(row):
        ident = re.search(r"(?:details/|driverResults.aspx/)(\d+)", row["details_url"] or "")
        if not ident:
            return [], {"source_url": WINDOWS_HISTORY, "reason": "DETAILS_ID_MISSING", "version": row["version"]}
        url = API + "?func=GetDownloadDetails&downloadID=" + ident[1]
        reply = client.request(url)
        if reply["status"] != 200:
            return [], {"source_url": url, "reason": "DETAILS_UNAVAILABLE", "version": row["version"]}
        rows = parse_driver_api(json.loads(reply["body"]))
        for d in rows:
            d.update(branch=row["branch"], release_date=row["release_date"], discovery_source_urls=[WINDOWS_HISTORY, url])
            d["branch_evidence"] = {"branch": row["branch"], "version": d["version"], "source_url": WINDOWS_HISTORY}
        return rows, None

    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        for rows, gap in pool.map(history_details, history):
            for row in rows:
                add(row)
            if gap:
                gaps.append(gap)

    # Public search API is bounded. Query each branch, both Game Ready and
    # Studio; record saturation and failure instead of claiming completeness.
    def search(pair):
        branch, studio = pair
        url = windows_search_url(branch, studio)
        reply = client.request(url)
        if reply["status"] != 200:
            return [], {"source_url": url, "reason": "SEARCH_UNAVAILABLE", "branch": branch}, []
        try:
            data = json.loads(reply["body"])
            rows = parse_driver_api(data)
        except (ValueError, KeyError):
            return [], {"source_url": url, "reason": "SEARCH_NO_RESULTS", "branch": branch}, []
        for d in rows:
            d["discovery_source_urls"] = [url]
            d["branch_evidence"]["source_url"] = url
        gap = {"source_url": url, "reason": "SEARCH_RESULT_LIMIT", "branch": branch} if len(data.get("IDS", [])) >= 20 else None
        excluded = [{"source_url": url, "version": x["downloadInfo"]["Version"], "reason": "API_ISWHQL_NOT_1"}
                    for x in data.get("IDS", []) if x.get("downloadInfo", {}).get("Version")
                    and x["downloadInfo"].get("IsWHQL") != "1"]
        return rows, gap, excluded

    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        for rows, gap, excluded in pool.map(search, [(b, s) for b in sorted(branch_meta) for s in (0, 1)]):
            exclusions.extend(excluded)
            for row in rows:
                if row["branch"] is None:
                    gaps.append({"source_url": row["discovery_source_urls"][0], "reason": "BRANCH_UNKNOWN", "version": row["version"]})
                else:
                    add(row)
            if gap:
                gaps.append(gap)
    for candidate in curated.get("extra_windows_candidates", []):
        add(candidate.copy())
    for d in candidates.values():
        branch_meta.setdefault(d["branch"], {"id": d["branch"], "branch_type": "UNKNOWN", "source_url": d["discovery_source_urls"][0], "type_scope": "UNKNOWN"})
    return list(branch_meta.values()), sorted(candidates.values(), key=lambda d: (d["branch"], d["os"], version_key(d["version"]))), gaps, exclusions


def collect(candidate, client, previous):
    attempts = []
    selected = None
    for url in candidate.get("candidate_urls", [candidate["url"]]):
        row = collect_installer({**candidate, "url": url}, client, previous.get((candidate["os"], candidate["version"])))
        attempts.append(row["probe"])
        if row["probe"]["status"] == 200:
            selected = row
            break
        # A 403/timeout is not absence; still probe a separately published path.
        if selected is None or selected["probe"]["status"] == 404 and row["probe"]["status"] != 404:
            selected = row
    selected.pop("candidate_urls", None)
    if selected["os"] == "windows":
        selected["product_line"] = "rtx-enterprise" if "quadro" in selected["url"].lower() else "geforce"
    selected["availability_probes"] = attempts
    return selected


def build_table(observations, curated):
    drivers = json.loads(json.dumps(observations["installers"]))
    branches = []
    for metadata in sorted(observations["branches"], key=lambda b: b["id"]):
        b = {**metadata, "name": f"R{metadata['id']}"}
        rows = [d for d in drivers if d["branch"] == b["id"]]
        # Bounds are discovered exact version extrema, not a historical-completeness claim.
        for key, choose in (("first_version", min), ("last_version", max)):
            b[key] = {os: choose((d["version"] for d in rows if d["os"] == os and d["probe"]["status"] == 200), key=version_key, default=None) for os in ("linux", "windows")}
        records = [r for r in curated["test_records"] if r["branch"] == b["id"]]
        b["test_records"] = records
        b["repo_status"] = "known-good" if any(r["result"] == "pass" for r in records) else "referenced-only" if records else "never-tested"
        branches.append(b)
    for d in drivers:
        records = [r for r in curated["test_records"] if r.get("os") == d["os"] and r.get("version") == d["version"]]
        d["test_records"] = records
        d["last_tested"] = max((r["date"] for r in records if r["result"] == "pass"), default=None)
    return {"schema_version": 1, "status": "DESIGN-ONLY", "generated_at": observations["observed_at"],
            "scope": curated["scope"], "families": curated["families"], "branches": branches,
            "known_bad": curated["known_bad"], "core_reference_versions": curated["core_reference_versions"],
            "abi_boundary_hypothesis": curated["abi_boundary_hypothesis"],
            "sources": observations["requests"], "discovery_gaps": observations["discovery_gaps"],
            "discovery_exclusions": observations.get("discovery_exclusions", []),
            "drivers": sorted(drivers, key=lambda d: (d["branch"], d["os"], version_key(d["version"])))}


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--offline", action="store_true")
    p.add_argument("--observations", type=Path, default=HERE / "observations.json")
    p.add_argument("--sources", type=Path, default=HERE / "sources.json")
    p.add_argument("--output", type=Path, default=HERE.parent / "driver-availability.json")
    p.add_argument("--cache", type=Path)
    p.add_argument("--timeout", type=int, default=20)
    p.add_argument("--hash-windows", action="append", default=[], metavar="VERSION")
    args = p.parse_args(argv)
    if len(args.hash_windows) > 3 or args.offline and args.hash_windows:
        p.error("hash at most three Windows installers, online only")
    curated = json.loads(args.sources.read_text())
    if args.offline:
        observations = json.loads(args.observations.read_text())
    else:
        client = Client(args.cache, args.timeout)
        branches, candidates, gaps, exclusions = discover(client, curated)
        previous = {(d["os"], d["version"]): d for d in json.loads(args.output.read_text())["drivers"]} if args.output.exists() else {}
        # A reviewed prior streamed hash is a pin receipt, never a fabricated digest.
        previous.update({(d["os"], d["version"]): d for d in curated.get("prior_windows_pins", [])})
        print(f"discovered {len(branches)} branches, {len(candidates)} OS/version candidates; probing HEAD", file=sys.stderr, flush=True)
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            installers = list(pool.map(lambda d: collect(d, client, previous), candidates))
        for ver in args.hash_windows:
            matches = [d for d in installers if d["os"] == "windows" and d["version"] == ver]
            if len(matches) != 1:
                raise ValueError("requested Windows exact version not discovered uniquely")
            if matches[0]["status"] != "PINNED":
                print(f"hashing Windows {ver}, one temporary file", file=sys.stderr, flush=True)
                hash_download(matches[0], client)
        observations = {"schema_version": 1, "observed_at": now(), "branches": branches, "installers": installers,
                        "requests": sorted(client.receipts.values(), key=lambda r: (r["url"], r["method"])), "discovery_gaps": gaps,
                        "discovery_exclusions": exclusions}
        write_json(args.observations, observations)
    table = build_table(observations, curated)
    write_json(args.output, table)
    print(f"wrote {args.output}: {len(table['drivers'])} installers, {sum(d['status'] == 'PINNED' for d in table['drivers'])} pins; {len(table['discovery_gaps'])} discovery gaps")
    return 0


if __name__ == "__main__":
    sys.exit(main())
