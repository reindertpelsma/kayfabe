#!/usr/bin/env python3
"""Pure stdlib-only official installer pin validation; no orchestration or network."""

import base64
import json
import re
import urllib.parse


def valid_driver_pin(pin):
    fields = {
        "os",
        "version",
        "status",
        "url",
        "sha256",
        "size",
        "checked",
        "source",
        "catalog_url",
    }
    if (
        not isinstance(pin, dict)
        or set(pin) != fields
        or pin["os"] not in ("linux", "windows")
        or not isinstance(pin["version"], str)
        or not re.fullmatch(r"[0-9]{3}\.[0-9]{2,3}(?:\.[0-9]{2})?", pin["version"])
        or pin["status"] not in ("PINNED", "UNPINNED")
        or not isinstance(pin["checked"], str)
        or not re.fullmatch(r"\d{4}-\d{2}-\d{2}", pin["checked"])
        or not isinstance(pin["source"], str)
        or not 1 <= len(pin["source"]) <= 512
    ):
        return False
    if pin["url"] is None:
        official = pin["status"] == "UNPINNED" and pin["catalog_url"] is None
    elif (
        isinstance(pin["url"], str)
        and len(pin["url"]) <= 2048
        and not re.search(r"[\s\\]", pin["url"])
    ):
        try:
            u = urllib.parse.urlsplit(pin["url"])
            official = u.scheme == "https" and not (
                u.username
                or u.password
                or u.port not in (None, 443)
                or u.query
                or u.fragment
            )
            if u.hostname in (
                "download.nvidia.com",
                "us.download.nvidia.com",
                "international.download.nvidia.com",
            ):
                official = (
                    official
                    and pin["catalog_url"] is None
                    and "/" + pin["version"] + "/" in u.path
                    and pin["version"] in u.path.rsplit("/", 1)[-1]
                    and u.path.endswith(".run" if pin["os"] == "linux" else ".exe")
                )
            else:
                official = (
                    official
                    and pin["os"] == "windows"
                    and u.hostname
                    in (
                        "catalog.s.download.windowsupdate.com",
                        "download.windowsupdate.com",
                    )
                    and isinstance(pin["catalog_url"], str)
                    and re.fullmatch(
                        r"https://www\.catalog\.update\.microsoft\.com/ScopedViewInline\.aspx\?updateid=[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
                        pin["catalog_url"],
                    )
                    is not None
                )
        except ValueError:
            return False
    else:
        return False
    if not official:
        return False
    if pin["status"] == "UNPINNED":
        return pin["sha256"] is None and pin["size"] is None
    return (
        isinstance(pin["sha256"], str)
        and re.fullmatch(r"[a-f0-9]{64}", pin["sha256"]) is not None
        and type(pin["size"]) is int
        and 0 < pin["size"] <= 8 * 1024**3
    )


def decode_driver_pins(value):
    if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,16384}", value):
        return None
    try:
        pins = json.loads(
            base64.b64decode(
                value + "=" * (-len(value) % 4), altchars=b"-_", validate=True
            )
        )
        if (
            not isinstance(pins, list)
            or not 1 <= len(pins) <= 128
            or not all(valid_driver_pin(p) for p in pins)
        ):
            return None
        if len({(p["os"], p["version"]) for p in pins}) != len(pins):
            return None
        return pins
    except (ValueError, TypeError, RecursionError):
        return None
