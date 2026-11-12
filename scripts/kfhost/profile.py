"""Profile rules. Everything here runs BEFORE anything is fetched or executed.

Wire format: the Worker is the source of truth (repo = canonical URL string, tree_key 64 hex, commit 40 hex,
stage names below). Profiles: owned | standard | container | borrowed. Only owned may ever run cmd frames.
"""

import re

from .config import PROFILES

# ONE constant: the stage names the Worker sends. Mapped to kfci steps in jobs.STAGE_TO_KFCI.
STAGES = ("provision", "cuda_host", "merge_bar", "cuda_guest", "apps", "windows_gate", "windows_apps",
          "specviewperf", "ladder")
HEX40 = re.compile(r"[0-9a-f]{40}\Z")
TREE_KEY = re.compile(r"[0-9a-f]{64}\Z")
IDENT = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_.:-]{0,127}\Z")
TOKEN = re.compile(r"[A-Za-z0-9._~+/=-]{1,4096}\Z")
OFFER_KEYS = {"type", "v", "offer_id", "request_id", "run_id", "run_token", "assignment", "deadline"}
BORROWED_ASSIGNMENT_KEYS = {"repo", "commit", "tree_key", "stages"}
OPEN_ASSIGNMENT_KEYS = BORROWED_ASSIGNMENT_KEYS | {"requirements", "env", "seed_url"}
# Epoch milliseconds; anything below this is seconds (or garbage) and is refused rather than misread.
MIN_DEADLINE_MS = 10**11
MAX_DEADLINE_MS = 4 * 10**12


class Refused(Exception):
    pass


def local_flags(cfg):
    return {"trust_git_only": cfg["trust_git_only"], "allow_cmd": cfg["allow_cmd"], "container": cfg["container"]}


def derive(local, welcome=None):
    """Effective profile name from the LOCAL flags and the Worker's welcome.

    Restrictions apply if either side sets them (the welcome may only ADD: profile borrowed or
    trust_git_only=true, profile container or container=true; anything else in it is ignored, so it can never
    loosen what the daemon enforces locally). Powers need both sides: owned = local allow_cmd AND the Worker's
    welcome profile owned. Before a welcome, no power is granted."""
    w = welcome if isinstance(welcome, dict) else {}
    if local["trust_git_only"] or w.get("profile") == "borrowed" or w.get("trust_git_only") is True:
        return "borrowed"
    if local["container"] or w.get("profile") == "container" or w.get("container") is True:
        return "container"
    if local["allow_cmd"] and w.get("profile") == "owned":
        return "owned"
    return "standard"


def needs_ancestry(profile):
    """Every profile but owned runs only commits reachable from the public master or next."""
    return profile != "owned"


def check_offer(profile, offer, cfg, now_ms):
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
    dl = offer.get("deadline")
    if type(dl) is not int or not MIN_DEADLINE_MS <= dl <= MAX_DEADLINE_MS:
        raise Refused("deadline must be an integer in epoch milliseconds")
    if dl <= now_ms:
        raise Refused("deadline already passed")
    a = offer.get("assignment")
    if not isinstance(a, dict):
        raise Refused("missing assignment")
    if profile == "borrowed" and any("seed" in k or "url" in k for k in a):
        raise Refused("borrowed never receives artifacts from other boxes (%s): build from public sources only"
                      % ",".join(sorted(k for k in a if "seed" in k or "url" in k)))
    allowed = BORROWED_ASSIGNMENT_KEYS if profile == "borrowed" else OPEN_ASSIGNMENT_KEYS
    if set(a) - allowed:
        raise Refused("fields not allowed for profile %s: %s" % (profile, ",".join(sorted(set(a) - allowed))))
    if a.get("repo") not in cfg["repo_allowlist"]:  # exact string match, every profile
        raise Refused("repo not in allowlist")
    if not (isinstance(a.get("commit"), str) and HEX40.fullmatch(a["commit"])):
        raise Refused("commit must be 40 lowercase hex")
    if not (isinstance(a.get("tree_key"), str) and TREE_KEY.fullmatch(a["tree_key"])):
        raise Refused("tree_key must be 64 lowercase hex")
    stages = a.get("stages", [])
    if not isinstance(stages, list) or len(stages) > 16 or not all(isinstance(s, str) and s in STAGES for s in stages):
        raise Refused("stage not in allowlist")
    if "seed_url" in a and not (isinstance(a["seed_url"], str) and re.fullmatch(r"https?://[A-Za-z0-9._:/-]{1,200}", a["seed_url"])):
        raise Refused("invalid seed_url")
    if "env" in a and not (
        isinstance(a["env"], dict)
        and all(isinstance(k, str) and re.fullmatch(r"KF_[A-Z0-9_]{1,60}", k) and isinstance(v, str) for k, v in a["env"].items())
    ):
        raise Refused("invalid env")
    return offer


def allows_cmd(profile, cfg, commands_until_ms, now_ms):
    """cmd frames: only profile owned AND local allow_cmd AND the Worker's active commands_until_ms."""
    return (
        profile == "owned"
        and cfg.get("allow_cmd") is True
        and type(commands_until_ms) is int
        and commands_until_ms > now_ms
    )


def standalone(profile):
    """borrowed runs make no system change (no sudo, no installs, no driver swap, official URLs only)."""
    return profile == "borrowed"
