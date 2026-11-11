"""Profile rules. Everything here runs BEFORE anything is fetched or executed."""

import re

from .config import PROFILES

# PROVISIONAL (sync with W1's borrowed-offer builder): the fixed stage names a borrowed offer may carry.
STAGE_ALLOWLIST = ("merge", "linux_apps", "windows", "winapps", "specviewperf", "ladder")
HEX40 = re.compile(r"[0-9a-f]{40}\Z")
TREE_KEY = re.compile(r"[0-9a-f]{40,64}\Z")
IDENT = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_.:-]{0,127}\Z")
TOKEN = re.compile(r"[A-Za-z0-9._~+/=-]{1,4096}\Z")
OFFER_KEYS = {"type", "v", "offer_id", "request_id", "run_id", "run_token", "assignment", "deadline"}
BORROWED_ASSIGNMENT_KEYS = {"repo", "commit", "tree_key", "stages"}
OPEN_ASSIGNMENT_KEYS = BORROWED_ASSIGNMENT_KEYS | {"requirements", "env"}


class Refused(Exception):
    pass


def effective(server_profile, ceiling):
    """The strictest of the server's profile and the local ceiling; unknown -> borrowed."""
    rank = {name: i for i, name in enumerate(PROFILES)}
    return PROFILES[min(rank.get(server_profile, 0), rank.get(ceiling, 0))]


def check_offer(profile, offer, cfg):
    """Return the validated offer or raise Refused(reason)."""
    if not isinstance(offer, dict) or set(offer) - OFFER_KEYS:
        raise Refused("unknown offer fields")
    for key in ("offer_id", "run_id"):
        if not isinstance(offer.get(key), str) or not IDENT.fullmatch(offer[key]):
            raise Refused("invalid " + key)
    if "request_id" in offer and not (isinstance(offer["request_id"], str) and IDENT.fullmatch(offer["request_id"])):
        raise Refused("invalid request_id")
    if not isinstance(offer.get("run_token"), str) or not TOKEN.fullmatch(offer["run_token"]):
        raise Refused("invalid run_token")
    a = offer.get("assignment")
    if not isinstance(a, dict):
        raise Refused("missing assignment")
    allowed = BORROWED_ASSIGNMENT_KEYS if profile == "borrowed" else OPEN_ASSIGNMENT_KEYS
    if set(a) - allowed:
        raise Refused("fields not allowed for profile %s: %s" % (profile, ",".join(sorted(set(a) - allowed))))
    repo = a.get("repo")
    if profile == "owned":
        if not (isinstance(repo, str) and repo.startswith("https://")):
            raise Refused("repo must be an https URL")
    elif repo not in cfg["repo_allowlist"]:
        raise Refused("repo not in allowlist")
    if not (isinstance(a.get("commit"), str) and HEX40.fullmatch(a["commit"])):
        raise Refused("commit must be 40 lowercase hex")
    if not (isinstance(a.get("tree_key"), str) and TREE_KEY.fullmatch(a["tree_key"])):
        raise Refused("invalid tree_key")
    stages = a.get("stages", [])
    allow = cfg.get("stage_allowlist") or STAGE_ALLOWLIST
    if not isinstance(stages, list) or len(stages) > 8 or not all(isinstance(s, str) and s in allow for s in stages):
        raise Refused("stage not in allowlist")
    if "env" in a and not (
        isinstance(a["env"], dict)
        and all(isinstance(k, str) and re.fullmatch(r"KF_[A-Z0-9_]{1,60}", k) and isinstance(v, str) for k, v in a["env"].items())
    ):
        raise Refused("invalid env")
    return offer


def allows_cmd(profile):
    return profile == "owned"


def standalone(profile):
    """borrowed runs make no system change (no sudo, no installs, no driver swap, official URLs only)."""
    return profile == "borrowed"
