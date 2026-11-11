"""Daemon configuration: a JSON file that must not be readable by group or others."""

import json
import os
import re
import stat

PROFILES = ("borrowed", "container", "owned")  # strictest first
PUBLIC_REPO = "https://github.com/reindertpelsma/kayfabe"
REQUIRED = ("worker_url", "box_id", "box_token", "access_id", "access_secret", "profile_ceiling", "work_dir")
OPTIONAL = {"repo_allowlist", "ws_path", "heartbeat_s", "stage_allowlist"}
BOX_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,63}\Z")


class ConfigError(ValueError):
    pass


def check_mode(path):
    st = os.stat(path)
    if not stat.S_ISREG(st.st_mode):
        raise ConfigError("%s is not a regular file" % path)
    if st.st_mode & 0o077:
        raise ConfigError(
            "%s is accessible by group/others (mode %o); run: chmod 600 %s" % (path, st.st_mode & 0o777, path)
        )


def load(path):
    check_mode(path)
    with open(path, encoding="utf-8") as stream:
        raw = json.load(stream)
    if not isinstance(raw, dict):
        raise ConfigError("config must be a JSON object")
    missing = [k for k in REQUIRED if not raw.get(k)]
    if missing:
        raise ConfigError("missing config fields: " + ", ".join(missing))
    unknown = set(raw) - set(REQUIRED) - OPTIONAL
    if unknown:
        raise ConfigError("unknown config fields: " + ", ".join(sorted(unknown)))
    if not all(isinstance(raw[k], str) for k in REQUIRED):
        raise ConfigError("config fields must be strings")
    if not raw["worker_url"].startswith("https://") or raw["worker_url"].endswith("/"):
        raise ConfigError("worker_url must be https:// without a trailing slash")
    if not BOX_ID.fullmatch(raw["box_id"]):
        raise ConfigError("invalid box_id")
    if raw["profile_ceiling"] not in PROFILES:
        raise ConfigError("profile_ceiling must be one of " + "|".join(PROFILES))
    cfg = dict(raw)
    cfg.setdefault("repo_allowlist", [PUBLIC_REPO])
    cfg.setdefault("ws_path", "/hosts/%s/ws" % cfg["box_id"])  # PROVISIONAL: W1 fixes the route
    cfg.setdefault("heartbeat_s", 15)
    if not isinstance(cfg["repo_allowlist"], list) or not all(isinstance(r, str) for r in cfg["repo_allowlist"]):
        raise ConfigError("repo_allowlist must be a list of URLs")
    return cfg


def ws_url(cfg):
    return "wss://" + cfg["worker_url"][len("https://") :] + cfg["ws_path"]
