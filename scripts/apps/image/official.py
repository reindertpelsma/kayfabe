# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Official-only NVIDIA acquisition. Never an R2/GitHub payload source."""

import importlib.util
import concurrent.futures
from pathlib import Path
import urllib.error
import urllib.request
from common import validate_pin, verify_file


class MediaDrift(ValueError):
    """Unavailable or changed pinned official media is never a passing stage."""


def check_url(pin, opener=None):
    validate_pin(pin)
    try:
        request = urllib.request.Request(
            pin["url"],
            method="HEAD",
            headers={"User-Agent": "kayfabe-official-verify/1.0"},
        )
        with (opener or urllib.request.urlopen)(request, timeout=45) as response:
            if response.status not in (200, 206):
                raise MediaDrift("MEDIA_DRIFT " + pin["id"] + ": HTTP status")
            length = response.headers.get("Content-Length")
            if length is not None and int(length) != pin["size"]:
                raise MediaDrift("MEDIA_DRIFT " + pin["id"] + ": length")
            return {
                "id": pin["id"],
                "status": response.status,
                "size": int(length) if length else None,
            }
    except (OSError, ValueError) as error:
        if isinstance(error, urllib.error.HTTPError):
            error.close()
        if isinstance(error, MediaDrift):
            raise
        raise MediaDrift(
            "MEDIA_DRIFT " + pin["id"] + ": official URL unavailable"
        ) from error


def verify_urls(pins, opener=None):
    if not pins:
        raise MediaDrift("MEDIA_DRIFT: official input list is empty")

    def check(pin):
        try:
            return check_url(pin, opener)
        except MediaDrift as error:
            return {"id": pin["id"], "status": "MEDIA_DRIFT", "error": str(error)}

    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        results = list(pool.map(check, pins))
    failed = [row["id"] for row in results if row["status"] == "MEDIA_DRIFT"]
    report = {
        "status": "MEDIA_DRIFT" if failed else "official-urls-answer",
        "results": results,
    }
    if failed:
        error = MediaDrift(
            "MEDIA_DRIFT: official URL verification failed: " + ", ".join(failed)
        )
        error.report = report
        raise error
    return report


def fetch(pin, directory, fetch_factory=None):
    validate_pin(pin)
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    target = directory / pin["sha256"]
    try:
        if target.exists():
            verify_file(target, pin)
            return target
        if fetch_factory is None:
            source = Path(__file__).resolve().parents[2] / "ci/kf_fetch.py"
            if not source.is_file():
                raise MediaDrift(
                    "MEDIA_DEPENDENCY: scripts/ci/kf_fetch.py (J4) is required"
                )
            spec = importlib.util.spec_from_file_location("apps_image_kf_fetch", source)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            fetch_factory = module.Download
        fetch_factory(
            pin["url"],
            target,
            pin["sha256"],
            streams=2,
            follow_redirect=True,
            expected_size=pin["size"],
            origin_headers={},
        ).run()
        verify_file(target, pin)
        return target
    except Exception as error:
        if isinstance(error, MediaDrift):
            raise
        raise MediaDrift(
            "MEDIA_DRIFT " + pin["id"] + ": official fetch/hash/size failure"
        ) from error
