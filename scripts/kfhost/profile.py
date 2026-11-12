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
OFFER_ID = re.compile(r"[0-9a-f]{16}\Z")
RUN_ID = re.compile(r"[1-9][0-9]{0,19}-[1-9][0-9]{0,19}\Z")  # <run>-<attempt>, as the Worker
REQUEST_ID = re.compile(r"[A-Za-z0-9_.:-]{1,64}\Z")
TOKEN = re.compile(r"[A-Za-z0-9._~+/=-]{1,1024}\Z")
SEED_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,99}\Z")
# `run` is PENDING (W1 is adding the run section to the offer): the nonces the result must carry.
OFFER_KEYS = {"type", "v", "offer_id", "request_id", "run_id", "run_token", "assignment", "deadline_ms", "run"}
RUN_KEYS = {"nonce", "runner_nonce", "sha", "request_id"}
SEED_KEYS = {"url", "fallback_url", "sha256", "size", "name"}
BORROWED_ASSIGNMENT_KEYS = {"repo", "commit", "tree_key", "stages"}
OPEN_ASSIGNMENT_KEYS = BORROWED_ASSIGNMENT_KEYS | {"requirements", "env", "seed"}
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
    """Only under --trust-git-only (profile borrowed): master/next-only fetch, ancestry check, tree key refusal.
    Every other profile does what the Worker says, including the commit."""
    return profile == "borrowed"


# The Worker's IP-encoding mappers (hosts.js MAPPERS, enabled ones): the fallback host is `<a-b-c-d>.<zone>`.
SEED_MAPPER_ZONES = ("sslip.io", "traefik.me")


def public_ipv4(value):
    """Dotted quad in public space, as the Worker's publicIpv4 (private, loopback, link-local, CGNAT, multicast,
    benchmarking and documentation ranges refused); None otherwise."""
    m = re.fullmatch(r"(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})", value) if isinstance(value, str) else None
    if not m or any(int(x) > 255 or str(int(x)) != x for x in m.groups()):
        return None
    a, b, c, _ = (int(x) for x in m.groups())
    if (a in (0, 10, 127) or a >= 224 or (a == 100 and 64 <= b <= 127) or (a == 169 and b == 254)
            or (a == 172 and 16 <= b <= 31) or (a == 192 and b == 168) or (a == 192 and b == 0 and c in (0, 2))
            or (a == 198 and b in (18, 19)) or (a == 198 and b == 51 and c == 100)
            or (a == 203 and b == 0 and c == 113) or (a == 192 and b == 88 and c == 99)):
        return None
    return value


def check_seed(sd):
    """The Worker's validateSeedRef: url http://<public ipv4>:<port>/<8-char token>/<name>; fallback_url (or null)
    the same path on `<a-b-c-d>.<mapper zone>` of the SAME ip and port. Anything else (private IP, other host,
    name mismatch) is refused: the daemon never fetches from an address the Worker would not contact."""
    if not (isinstance(sd, dict) and set(sd) <= SEED_KEYS):
        raise Refused("invalid seed")
    name = sd.get("name")
    if not (isinstance(name, str) and SEED_NAME.fullmatch(name) and ".." not in name
            and isinstance(sd.get("sha256"), str) and re.fullmatch(r"[0-9a-f]{64}", sd["sha256"])
            and type(sd.get("size")) is int and sd["size"] >= 1):
        raise Refused("invalid seed")
    u = re.fullmatch(r"http://([0-9.]+):([0-9]{1,5})/([a-z0-9]{8})/" + re.escape(name), sd.get("url") or "")
    if not (u and public_ipv4(u.group(1)) and 1 <= int(u.group(2)) <= 65535):
        raise Refused("invalid seed url (needs a public IPv4)")
    fb = sd.get("fallback_url")
    if fb is not None:
        zones = "|".join(re.escape(z) for z in SEED_MAPPER_ZONES)
        f = re.fullmatch(r"http://([0-9]{1,3}(?:-[0-9]{1,3}){3})\.(?:" + zones + r"):" + u.group(2) + "/" + u.group(3) + "/"
                         + re.escape(name), fb) if isinstance(fb, str) else None
        if not (f and f.group(1).replace("-", ".") == u.group(1)):
            raise Refused("invalid seed fallback_url")


def check_offer(profile, offer, cfg, now_ms):
    """Return the validated offer or raise Refused(reason)."""
    if not isinstance(offer, dict) or set(offer) - OFFER_KEYS:
        raise Refused("unknown offer fields")
    for key, rx in (("offer_id", OFFER_ID), ("run_id", RUN_ID)):
        if not isinstance(offer.get(key), str) or not rx.fullmatch(offer[key]):
            raise Refused("invalid " + key)
    if not (isinstance(offer.get("request_id"), str) and REQUEST_ID.fullmatch(offer["request_id"])):
        raise Refused("invalid request_id")
    if not isinstance(offer.get("run_token"), str) or not TOKEN.fullmatch(offer["run_token"]):
        raise Refused("invalid run_token")
    dl = offer.get("deadline_ms")
    if type(dl) is not int or not MIN_DEADLINE_MS <= dl <= MAX_DEADLINE_MS:
        raise Refused("deadline_ms must be an integer in epoch milliseconds")
    if dl <= now_ms:
        raise Refused("deadline_ms already passed")
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
    if not isinstance(stages, list) or len(stages) > 32 or not all(isinstance(s, str) and s in STAGES for s in stages):
        raise Refused("stage not in allowlist")
    if "seed" in a:
        check_seed(a["seed"])
    run = offer.get("run")  # required by the Worker's contract: nonce, runner_nonce, sha (the result echoes them)
    if not (isinstance(run, dict) and set(run) <= RUN_KEYS
            and re.fullmatch(r"[0-9a-f]{32}", str(run.get("nonce", "")))
            and re.fullmatch(r"[0-9a-f]{64}", str(run.get("runner_nonce", "")))
            and run.get("sha") == a["commit"]
            and ("request_id" not in run or run["request_id"] == offer["request_id"])):
        raise Refused("invalid or missing run section (nonce, runner_nonce, sha)")
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
