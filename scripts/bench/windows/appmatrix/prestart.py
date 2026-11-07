# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Optional guest prestart descriptor; no commands or arbitrary script paths."""
import re

SCRIPT = r"C:\kf\app_prestart.ps1"


def config(app):
    if "prestart" not in app:
        return None
    raw = app["prestart"]
    if not isinstance(raw, dict) or set(raw) - {"timeout_s"}:
        raise ValueError("invalid prestart fields")
    if not re.fullmatch(r"[a-z][a-z0-9_]{0,63}", app["id"]):
        raise ValueError("invalid prestart app id")
    seconds = raw.get("timeout_s", 30)
    if type(seconds) is not int or not 1 <= seconds <= 120:
        raise ValueError("invalid prestart timeout")
    return dict(timeout_s=seconds)


def arguments(app, spec, action):
    return ["-App", app["id"], "-Action", action, "-TimeoutSec", str(spec["timeout_s"])]


def succeeded(result, app, action):
    marker = f"APP_PRESTART_DONE app={app['id']} action={action}"
    return result.exitcode == 0 and marker in result.text().splitlines()
