#!/usr/bin/env python3
"""One-off cache builder; no rental, private-store credentials or code compilation."""

import concurrent.futures
import hashlib
import json
import math
from pathlib import Path, PurePosixPath
import re
import shlex
import subprocess
import sys
import tarfile
import time
import urllib.parse
import urllib.request

from vast_setup import CacheRefused, NoRedirect, LOCK_PATHS, PLATFORM, TARGET_IMAGE
from vast_setup import digest, package_names, policy_digest

MIRRORS = ("archive.ubuntu.com", "azure.archive.ubuntu.com", "mirrors.edge.kernel.org")
APT_HOSTS = set(MIRRORS) | {"security.ubuntu.com"}


def parse_uris(text, metadata):
    rows, seen = [], set()
    try:
        for line in text.splitlines():
            if not line.startswith("'"):
                continue
            parts = shlex.split(line)
            if len(parts) != 4:
                raise ValueError()
            url, name, size, _ = parts
            parsed = urllib.parse.urlsplit(url)
            if (
                parsed.scheme not in ("http", "https")
                or parsed.hostname not in APT_HOSTS
                or parsed.username
                or parsed.password
                or parsed.port
                or parsed.query
                or parsed.fragment
                or not re.fullmatch(r"[A-Za-z0-9+_.:%~-]+\.deb", name)
                or name in seen
            ):
                raise ValueError()
            package, version, arch = urllib.parse.unquote(name[:-4]).rsplit("_", 2)
            control = metadata(package, version, arch)
            if (
                control["Package"] != package
                or control["Version"] != version
                or control["Architecture"] != arch
                or int(control["Size"]) != int(size)
                or not re.fullmatch(r"[0-9a-f]{64}", control["SHA256"])
                or urllib.parse.unquote(parsed.path) != "/ubuntu/" + control["Filename"]
                or not control["Filename"].startswith("pool/")
                or ".." in PurePosixPath(control["Filename"]).parts
            ):
                raise ValueError()
            name = PurePosixPath(control["Filename"]).name
            if name in seen:
                raise ValueError()
            seen.add(name)
            rows.append(
                {"name": name, "size": int(size), "sha256": control["SHA256"], "control": control}
            )
        if not rows:
            raise ValueError()
        return rows
    except (KeyError, TypeError, ValueError):
        raise CacheRefused("invalid signed APT plan") from None


def rank_probes(rows):
    valid = []
    for row in rows:
        try:
            if (
                row["host"] in MIRRORS
                and row["status"] in (200, 206)
                and type(row["bytes"]) in (int, float)
                and row["bytes"] > 0
                and type(row["seconds"]) in (int, float)
                and row["seconds"] > 0
                and math.isfinite(row["seconds"])
                and math.isfinite(row["bytes"])
            ):
                valid.append(
                    (row["bytes"] / row["seconds"], -MIRRORS.index(row["host"]), row["host"])
                )
        except (KeyError, TypeError, ValueError):
            continue
    if not valid:
        raise CacheRefused("no usable APT mirror")
    return max(valid)[2]


def probe_mirrors(row, require=True):
    def probe(host):
        url = "https://" + host + "/ubuntu/" + row["control"]["Filename"]
        p = subprocess.run(
            [
                "curl",
                "--silent",
                "--fail",
                "--proto",
                "=https",
                "--max-redirs",
                "0",
                "--max-time",
                "10",
                "--range",
                "0-8388607",
                "--output",
                "/dev/null",
                "--write-out",
                "%{http_code} %{size_download} %{time_total}",
                url,
            ],
            capture_output=True,
            text=True,
            timeout=12,
        )
        try:
            code, size, seconds = p.stdout.split()
            return {
                "host": host,
                "status": int(code),
                "bytes": int(float(size)),
                "seconds": float(seconds),
                "curl_rc": p.returncode,
            }
        except ValueError:
            return {"host": host, "status": 0, "bytes": 0, "seconds": 0, "curl_rc": p.returncode}

    with concurrent.futures.ThreadPoolExecutor(3) as pool:
        rows = list(pool.map(probe, MIRRORS))
    try:
        winner = rank_probes(rows)
    except CacheRefused:
        if require:
            raise
        winner = None
    return winner, rows


def fetch_deb(row, folder, mirror, *, opener=None):
    path = Path(folder) / row["name"]
    temporary = path.with_suffix(".partial")
    try:
        if mirror not in MIRRORS:
            raise ValueError()
        url = "https://" + mirror + "/ubuntu/" + row["control"]["Filename"]
        if opener is None:
            opener = urllib.request.build_opener(NoRedirect())
        request = urllib.request.Request(url, headers={})
        size, h = 0, hashlib.sha256()
        with opener.open(request, timeout=30) as source, temporary.open("xb") as out:
            if source.status != 200:
                raise ValueError()
            while chunk := source.read(1024 * 1024):
                size += len(chunk)
                if size > row["size"]:
                    raise ValueError()
                h.update(chunk)
                out.write(chunk)
        if size != row["size"] or h.hexdigest() != row["sha256"]:
            raise ValueError()
        temporary.replace(path)
        return path
    except Exception:
        temporary.unlink(missing_ok=True)
        raise CacheRefused("APT prefetch refused") from None


def control_metadata(package, version, arch):
    text = subprocess.check_output(["apt-cache", "show", package + "=" + version], text=True)
    for block in text.split("\n\n"):
        values, previous = {}, None
        for line in block.splitlines():
            if line.startswith(" ") and previous:
                values[previous] += "\n" + line
            elif ": " in line:
                previous, value = line.split(": ", 1)
                values[previous] = value
        if values.get("Version") == version and values.get("Architecture") == arch:
            return values
    raise CacheRefused("APT metadata missing")


def pack(root, kind, destination):
    root, destination = Path(root), Path(destination)
    total, count = 0, 0
    with tarfile.open(destination, "w:gz", compresslevel=6) as tar:
        for path in [root, *sorted(root.rglob("*"))]:
            if path.is_symlink() or not (path.is_file() or path.is_dir()):
                raise CacheRefused("unsupported cache file")
            arcname = str(Path(kind) / path.relative_to(root))
            info = tar.gettarinfo(str(path), arcname=arcname)
            info.uid = info.gid = info.mtime = 0
            info.uname = info.gname = ""
            info.mode = 0o755 if path.is_dir() or info.mode & 0o111 else 0o644
            if path.is_file():
                with path.open("rb") as data:
                    tar.addfile(info, data)
                total += info.size
                count += 1
            else:
                tar.addfile(info)
    size = destination.stat().st_size
    if not 0 < size <= 2_000_000_000:
        raise CacheRefused("split cache asset before publishing")
    return {"kind": kind, "name": destination.name, "size": size, "sha256": digest(destination)}, {
        "files": count,
        "unpacked_bytes": total,
    }


def build_debs(root, out):
    started = time.time()
    subprocess.run(["apt-get", "update", "-qq"], check=True)
    names = package_names(root)
    command = [
        "apt-get",
        "-o",
        "Dir::State::status=/dev/null",
        "--no-install-recommends",
        "--download-only",
        "--print-uris",
        "--yes",
        "install",
        *names,
    ]
    plan = subprocess.check_output(command, text=True)
    rows = parse_uris(plan, control_metadata)
    mirror, probes = probe_mirrors(max(rows, key=lambda x: x["size"]))
    folder = out / "debs"
    folder.mkdir(parents=True, exist_ok=False)
    at = time.time()
    with concurrent.futures.ThreadPoolExecutor(6) as pool:
        list(pool.map(lambda row: fetch_deb(row, folder, mirror), rows))
    prefetch_seconds = time.time() - at
    paragraphs = []
    for row in rows:
        control = dict(row["control"])
        control.update(Filename="./" + row["name"], Size=str(row["size"]), SHA256=row["sha256"])
        paragraphs.append("\n".join(k + ": " + v for k, v in control.items()))
    (folder / "Packages").write_text("\n\n".join(paragraphs) + "\n")
    manifest = [
        {k: row["control"][k] for k in ["Package", "Version", "Architecture"]}
        | {
            "name": row["name"],
            "upstream_path": row["control"]["Filename"],
            "size": row["size"],
            "sha256": row["sha256"],
        }
        for row in rows
    ]
    (folder / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    stats = {
        "run": "v8-cache-build-20261023",
        "packages": len(rows),
        "deb_bytes": sum(x["size"] for x in rows),
        "mirror": mirror,
        "probes": probes,
        "prefetch_workers": 6,
        "prefetch_seconds": prefetch_seconds,
        "builder_seconds": time.time() - started,
    }
    (out / "deb-build-stats.json").write_text(json.dumps(stats, indent=2) + "\n")
    asset, unpacked = pack(folder, "debs", out / "debs.tar.gz")
    (out / "deb-asset.json").write_text(
        json.dumps({"asset": asset, "contents": unpacked}, indent=2)
    )
    print(json.dumps(stats), flush=True)


def make_lock(root, out, tag):
    deb = json.loads((out / "deb-asset.json").read_text())["asset"]
    vendor, contents = pack(out / "vendor", "vendor", out / "vendor.tar.gz")
    lock = {
        "schema": 1,
        "image": TARGET_IMAGE,
        "platform": PLATFORM,
        "package_policy_sha256": policy_digest(root),
        "locks": {name: digest(root / name) for name in LOCK_PATHS},
        "tag": tag,
        "assets": [deb, vendor],
    }
    (out / "setup-cache.lock.json").write_text(json.dumps(lock, indent=2) + "\n")
    (out / "vendor-asset.json").write_text(
        json.dumps({"asset": vendor, "contents": contents}, indent=2)
    )
    print(json.dumps({"assets": lock["assets"], "vendor": contents}), flush=True)


def main():
    if sys.argv[1] == "debs":
        build_debs(Path(sys.argv[2]), Path(sys.argv[3]))
    elif sys.argv[1] == "probe":
        cache = Path(sys.argv[2])
        manifest = json.loads((cache / "debs/manifest.json").read_text())
        largest = max(manifest, key=lambda row: row["size"])
        mirror, probes = probe_mirrors(
            {"control": {"Filename": largest["upstream_path"]}}, require=False
        )
        metrics = {"fallback_mirror": mirror, "probes": probes, "actual_apt_source": "local-cache"}
        (cache / "mirror-metrics.json").write_text(json.dumps(metrics) + "\n")
        print("KF_SETUP_MIRRORS " + json.dumps(metrics, separators=(",", ":")), flush=True)
    elif sys.argv[1] == "lock":
        make_lock(Path(sys.argv[2]), Path(sys.argv[3]), sys.argv[4])
    else:
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
