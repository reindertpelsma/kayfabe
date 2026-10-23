#!/usr/bin/env python3
"""Reviewed dependency cache: scoped blob URLs, verified bytes, local APT and Cargo sources."""

import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.parse
import urllib.request

TARGET_IMAGE = "docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16"
PLATFORM = {"os": "ubuntu", "version": "22.04", "arch": "x86_64"}
LOCK_PATHS = ("Cargo.lock", "firmware/kf-gop/Cargo.lock")
CACHE_ROOT = Path("/workspace/kf-setup-cache")
GITHUB_HOSTS = {
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
    "github-releases.githubusercontent.com",
}


class CacheRefused(ValueError):
    """Generic refusal; signed locators and remote contents never reach the error text."""


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def package_names(root):
    names = set()
    for profile in ["ci", "bench"]:
        text = subprocess.check_output(
            ["bash", str(root / "scripts/bench/provision_box.sh"), "--packages", profile], text=True
        )
        names.update(text.split())
    if not names or not all(re.fullmatch(r"[a-z0-9][a-z0-9+.-]*", x) for x in names):
        raise CacheRefused("invalid package policy")
    return sorted(names)


def policy_digest(root):
    return hashlib.sha256(
        json.dumps(package_names(root), separators=(",", ":")).encode()
    ).hexdigest()


def validate_lock(lock, root, facts, policy):
    try:
        if lock["schema"] != 1 or lock["image"] != TARGET_IMAGE:
            raise ValueError()
        if facts != PLATFORM or lock["platform"] != facts:
            raise ValueError()
        if lock["package_policy_sha256"] != policy:
            raise ValueError()
        if set(lock["locks"]) != set(LOCK_PATHS):
            raise ValueError()
        if any(lock["locks"][name] != digest(root / name) for name in LOCK_PATHS):
            raise ValueError()
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}", lock["tag"]):
            raise ValueError()
        assets = lock["assets"]
        if len(assets) != 2 or {a["kind"] for a in assets} != {"debs", "vendor"}:
            raise ValueError()
        for asset in assets:
            if (
                set(asset) != {"kind", "name", "size", "sha256"}
                or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,99}", asset["name"])
                or type(asset["size"]) is not int
                or not 0 < asset["size"] <= 2_000_000_000
                or not re.fullmatch(r"[0-9a-f]{64}", asset["sha256"])
            ):
                raise ValueError()
        if len({a["name"] for a in assets}) != 2:
            raise ValueError()
        return lock
    except (KeyError, TypeError, ValueError, OSError):
        raise CacheRefused("setup cache identity mismatch") from None


def download_asset(locator, asset, destination, *, opener=None):
    destination = Path(destination)
    temporary = destination.with_suffix(".partial")
    try:
        if set(locator) != {"url", "size", "name"}:
            raise ValueError()
        if locator["size"] != asset["size"] or locator["name"] != asset["name"]:
            raise ValueError()
        url = urllib.parse.urlsplit(locator["url"])
        if (
            url.scheme != "https"
            or url.hostname not in GITHUB_HOSTS
            or url.username
            or url.password
            or url.port
            or url.fragment
            or len(locator["url"]) > 8192
        ):
            raise ValueError()
        if opener is None:
            opener = urllib.request.build_opener(NoRedirect())
        request = urllib.request.Request(locator["url"], headers={}, method="GET")
        size, h = 0, hashlib.sha256()
        with opener.open(request, timeout=30) as response, temporary.open("xb") as output:
            if response.status != 200:
                raise ValueError()
            while chunk := response.read(1024 * 1024):
                size += len(chunk)
                if size > asset["size"]:
                    raise ValueError()
                h.update(chunk)
                output.write(chunk)
        if size != asset["size"] or h.hexdigest() != asset["sha256"]:
            raise ValueError()
        temporary.replace(destination)
    except Exception:
        temporary.unlink(missing_ok=True)
        raise CacheRefused("setup cache download refused") from None


def unpack_checked(archive, destination, kind, *, limit=8_000_000_000):
    destination = Path(destination)
    created = False
    try:
        destination.mkdir(parents=True, exist_ok=False)
        created = True
        with tarfile.open(archive, "r:gz") as tar:
            members = tar.getmembers()
            seen, total = set(), 0
            if not 0 < len(members) <= 200000:
                raise ValueError()
            for member in members:
                path = PurePosixPath(member.name)
                if (
                    path.is_absolute()
                    or ".." in path.parts
                    or "\\" in member.name
                    or path.parts[0] != kind
                    or member.name in seen
                    or not (member.isfile() or member.isdir())
                    or member.mode & 0o7000
                ):
                    raise ValueError()
                seen.add(member.name)
                total += member.size
                if total > limit:
                    raise ValueError()
            for member in members:
                target = destination / member.name
                if member.isdir():
                    target.mkdir(parents=True, exist_ok=True)
                    target.chmod(0o755)
                else:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with tar.extractfile(member) as source, target.open("xb") as output:
                        shutil.copyfileobj(source, output)
                    target.chmod(0o755 if member.mode & 0o111 else 0o644)
    except Exception:
        if created:
            shutil.rmtree(destination, ignore_errors=True)
        raise CacheRefused("setup cache archive refused") from None


def configure_vendor(vendor, cargo_home):
    home = Path(cargo_home)
    home.mkdir(parents=True, exist_ok=True)
    path = home / "config.toml"
    before = path.read_text() if path.exists() else ""
    block = (
        '\n[source.crates-io]\nreplace-with = "kf-setup-vendor"\n'
        "[source.kf-setup-vendor]\ndirectory = " + json.dumps(str(vendor)) + "\n"
    )
    if (home / "config").exists() or ("[source" in before and not before.endswith(block)):
        raise CacheRefused("existing Cargo source override")
    if not before.endswith(block):
        path.write_text(before + block)


def apt_command(cache, arguments):
    cache = Path(cache)
    return [
        "/usr/bin/apt-get",
        *arguments,
        "-o",
        "APT::Install-Recommends=false",
        "-o",
        "APT::Install-Suggests=false",
        "-o",
        "Acquire::Queue-Mode=host",
        "-o",
        "Acquire::http::Pipeline-Depth=10",
        "-o",
        "Acquire::https::Pipeline-Depth=10",
        "-o",
        "Dir::Etc::sourcelist=" + str(cache / "sources.list"),
        "-o",
        "Dir::Etc::sourceparts=-",
        "-o",
        "Dir::State::lists=" + str(cache / "lists"),
    ]


def apt(cache, arguments):
    cache = Path(cache)
    started = time.time()
    with (cache / "apt-install.log").open("ab") as log:
        rc = subprocess.run(apt_command(cache, arguments), stdout=log, stderr=log).returncode
    entry = {
        "operation": next((x for x in arguments if not x.startswith("-")), "unknown"),
        "rc": rc,
        "seconds": round(time.time() - started, 3),
    }
    with (cache / "apt-metrics.jsonl").open("a") as out:
        out.write(json.dumps(entry) + "\n")
    print("KF_SETUP_APT " + json.dumps(entry, separators=(",", ":")), flush=True)
    return rc


def wait_for_ready():
    """Wait for the reporter acknowledgement event; the existing payload deadline bounds it."""
    fd = None
    try:
        value = os.environ["KF_BROKER_READY_FD"]
        if not re.fullmatch(r"[1-9][0-9]{0,5}", value) or not 3 <= int(value) <= 999999:
            raise ValueError()
        fd = int(value)
        if os.read(fd, 16) != b"READY\n":
            raise ValueError()
    except (KeyError, OSError, ValueError):
        raise CacheRefused("broker acknowledgement unavailable") from None
    finally:
        if fd is not None:
            try:
                os.close(fd)
            except OSError:
                pass


def restore(root, cache=CACHE_ROOT):
    root, cache = Path(root), Path(cache)
    started = time.time()
    print("KF_SETUP_CACHE_START utc=" + str(int(started)), flush=True)
    facts = {}
    for line in Path("/etc/os-release").read_text().splitlines():
        key, sep, value = line.partition("=")
        if sep and key in ["ID", "VERSION_ID"]:
            facts["os" if key == "ID" else "version"] = value.strip('"')
    facts["arch"] = platform.machine()
    lock = json.loads((root / "scripts/ci/setup-cache.lock.json").read_text())
    validate_lock(lock, root, facts, policy_digest(root))
    wait_for_ready()
    cache.mkdir(parents=True, exist_ok=False)
    for asset in lock["assets"]:
        at = time.time()
        raw = subprocess.check_output(
            ["/usr/local/bin/kf-box-blob-url", lock["tag"], asset["name"]],
            stderr=subprocess.DEVNULL,
        )
        if len(raw) > 16384:
            raise CacheRefused("invalid blob locator")
        locator = json.loads(raw)
        archive = cache / asset["name"]
        download_asset(locator, asset, archive)
        staging = cache / ("staging-" + asset["kind"])
        unpack_checked(archive, staging, asset["kind"])
        (staging / asset["kind"]).replace(cache / asset["kind"])
        staging.rmdir()
        archive.unlink()
        print(
            "KF_SETUP_ASSET kind="
            + asset["kind"]
            + " bytes="
            + str(asset["size"])
            + " sha256="
            + asset["sha256"]
            + " seconds="
            + str(round(time.time() - at, 3)),
            flush=True,
        )
    subprocess.run(
        ["python3", str(root / "scripts/ci/setup_cache_build.py"), "probe", str(cache)], check=True
    )
    (cache / "lists/partial").mkdir(parents=True)
    cache.chmod(0o755)
    (cache / "sources.list").write_text("deb [trusted=yes] file:" + str(cache / "debs") + " ./\n")
    # Existing provision scripts mask unattended-upgrade timers and guard the dpkg lock.
    # They install through the cache-only apt shim; restoration never bypasses that order.
    configure_vendor(cache / "vendor", Path.home() / ".cargo")
    metrics = {
        "seconds": round(time.time() - started, 3),
        "package_policy_sha256": policy_digest(root),
        "asset_bytes": sum(a["size"] for a in lock["assets"]),
    }
    (cache / "setup-metrics.json").write_text(json.dumps(metrics) + "\n")
    print("KF_SETUP_CACHE_END " + json.dumps(metrics, separators=(",", ":")), flush=True)


def main():
    try:
        if sys.argv[1] == "apt":
            return apt(Path(os.environ["KF_APT_CACHE_ROOT"]), sys.argv[2:])
        if sys.argv[1] == "restore" and len(sys.argv) == 3:
            restore(Path(sys.argv[2]))
            return 0
        return 2
    except Exception:
        # Never print exception details: urllib and subprocess errors can carry a signed locator.
        result = {
            "verdict": "FAIL",
            "suites": [{"name": "setup_cache", "pass": 0, "fail": 1, "skip": 0}],
            "hardware": {"gpu_name": "unknown", "compute_cap": 0, "driver_version": "unknown"},
        }
        workspace = Path(os.environ.get("KF_CI_WORKSPACE", "/workspace"))
        workspace.mkdir(parents=True, exist_ok=True)
        (workspace / "kf-result.json").write_text(json.dumps(result) + "\n")
        print("KF_SETUP_CACHE_REFUSED", flush=True)
        return 2


if __name__ == "__main__":
    sys.exit(main())
