"""Daemon configuration: a JSON file that must not be readable by group or others."""

import json
import os
import re
import stat

PROFILES = ("borrowed", "container", "standard", "owned")  # derived names (profile.derive); only owned may run cmd
PUBLIC_REPO = "https://github.com/reindertpelsma/kayfabe"
REQUIRED = ("worker_url", "box_id", "box_token", "access_id", "access_secret", "work_dir")
# Local flags. trust_git_only and container are RESTRICTIONS (the Worker may add them, never remove them);
# allow_cmd is a POWER (needs the Worker to enable it as well). Default: standard.
FLAGS = ("trust_git_only", "allow_cmd", "container")
OPTIONAL = {"repo_allowlist", "ws_path", "heartbeat_s", "distrust_us"} | set(FLAGS)
BOX_ID = re.compile(r"[a-z0-9][a-z0-9-]{0,39}\Z")  # the Worker's format


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
    cfg = dict(raw)
    if "distrust_us" in cfg:  # old name of trust_git_only
        cfg["trust_git_only"] = cfg.pop("distrust_us")
    cfg.setdefault("repo_allowlist", [PUBLIC_REPO])
    cfg.setdefault("ws_path", "/hosts/%s/ws" % cfg["box_id"])  # PROVISIONAL: W1 fixes the route
    cfg.setdefault("heartbeat_s", 15)
    for flag in FLAGS:
        cfg.setdefault(flag, False)
        if cfg[flag] is not True and cfg[flag] is not False:
            raise ConfigError("%s must be true or false" % flag)
    if not isinstance(cfg["repo_allowlist"], list) or not all(isinstance(r, str) for r in cfg["repo_allowlist"]):
        raise ConfigError("repo_allowlist must be a list of URLs")
    if not all(r.startswith("https://") and not r.endswith("/") for r in cfg["repo_allowlist"]):
        raise ConfigError("repo_allowlist entries must be canonical https:// URLs without a trailing slash")
    return cfg


def ws_url(cfg):
    return "wss://" + cfg["worker_url"][len("https://") :] + cfg["ws_path"]
