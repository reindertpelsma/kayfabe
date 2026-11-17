#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Package manifest, offline enforcement and runtime injection for the Linux proprietary image (propimage_linux.py).

STATUS: LIVE as tooling, 2026-11-17 (GPU-free; unit-tested with scripted guests, NOT run on a guest). Design:
docs/design/V3_PROPIMAGE_LINUX.md section 13. Pure functions; every guest action goes through a `ssh(remote, stdin=None,
timeout=600)` callable given by the caller (propimage_linux.py wires its own ssh transport), so nothing here boots or touches a VM.

Owner rules, 2026-11-17: (1) no apt and no other network inside offline clones (apt only in the SETUP VM before the seal);
(2) every package of the sealed image is documented (packages.manifest.json); (3) no Python dependencies in the image: Python
environments are built at run time and copied into the running offline clone.
"""
import hashlib
import json
import os
import posixpath
import re
import shlex
import subprocess
import tarfile
import tempfile
from pathlib import Path

SCHEMA = "kayfabe-propimage-linux-packages/v1"
MANIFEST_NAME = "packages.manifest.json"
APT_RECORD_NAME = "apt_sources.recorded.json"
HARDEN_RECORD_NAME = "hardening.json"
LARGE_DIR_BYTES = 1 << 30
DEFAULT_MAX_INJECT_GB = 20.0
INJECT_EXTRA_ROOT = "/opt/kf"
MASK_UNITS = ("apt-daily.timer", "apt-daily-upgrade.timer", "apt-daily.service", "apt-daily-upgrade.service", "unattended-upgrades.service")


class PkgError(Exception):
    """A named refusal: the message starts with an upper-case code (VENDOR_ENTRY_BAD, HARDEN_EXCEPTION, INJECT_DEST_REFUSED, ...)."""


def require(cond, msg):
    if not cond:
        raise PkgError(msg)


def sha256_bytes(b):
    return hashlib.sha256(b).hexdigest()


# ---------------------------------------------------------------- vendor entries (free-form, from the command line)
SECRET_KEY_RE = re.compile(r"(pass|token|secret|credential|licen[sc]e|activation|api_?key|^key$|_key$)", re.I)
CRED_VALUE_RE = re.compile(r"://[^/\s]*:[^/\s]*@")


def parse_vendor_entry(text):
    """`name=MATLAB release=R2026b products=A,B installer_sha256=<h> installed_via=mpm` -> dict. Free-form keys (lowercase),
    `products` becomes a list, `installer_sha256` must be 64 hex. A key that names a secret, or a URL with credentials, is refused:
    the manifest is a public-shaped record, never a place for sign-in data."""
    try:
        toks = shlex.split(text)
    except ValueError as e:
        raise PkgError("VENDOR_ENTRY_BAD: %s" % e)
    require(toks, "VENDOR_ENTRY_BAD: empty entry")
    ent = {}
    for t in toks:
        k, eq, v = t.partition("=")
        require(eq and re.fullmatch(r"[a-z][a-z0-9_]{0,40}", k), "VENDOR_ENTRY_BAD: %r is not key=value with a lowercase key" % t[:40])
        require(k not in ent, "VENDOR_ENTRY_BAD: duplicate key " + k)
        require(not SECRET_KEY_RE.search(k), "VENDOR_ENTRY_SECRET_KEY: key %r looks like a secret; the manifest never carries one" % k)
        require(v and len(v) <= 512 and v.isprintable(), "VENDOR_ENTRY_BAD: empty, long or non-printable value for " + k)
        require(not CRED_VALUE_RE.search(v), "VENDOR_ENTRY_CREDENTIAL: the value of %s holds a URL with credentials" % k)
        ent[k] = v
    require("name" in ent, "VENDOR_ENTRY_BAD: an entry needs name=...")
    if "installer_sha256" in ent:
        ent["installer_sha256"] = ent["installer_sha256"].lower()
        require(re.fullmatch(r"[0-9a-f]{64}", ent["installer_sha256"]), "VENDOR_ENTRY_BAD: installer_sha256 must be 64 hex characters")
    if "products" in ent:
        ent["products"] = [x for x in ent["products"].split(",") if x]
    return ent


# ---------------------------------------------------------------- guest collector (raw outputs; parsing happens on the host)
GUEST_RUNNER_PY = r"""
import json, subprocess, sys
cmds = json.loads(sys.stdin.read())
out = {}
for k, c in cmds.items():
    try:
        p = subprocess.run(c, shell=True, capture_output=True, text=True, timeout=600)
        out[k] = {"rc": p.returncode, "out": p.stdout}
    except Exception as e:
        out[k] = {"rc": -1, "out": ""}
print(json.dumps(out))
"""

PIP_PY = """python3 - <<'PYEOF'
import json, importlib.metadata as m
r = []
for d in m.distributions():
    loc = str(d.locate_file(""))
    if loc.startswith("/usr/lib/python3/"):
        continue   # dpkg-owned: listed under dpkg
    r.append({"name": d.metadata["Name"], "version": d.version, "location": loc})
print(json.dumps(r))
PYEOF"""

APT_FILES_PY = """python3 - <<'PYEOF'
import glob, json, os
paths = ["/etc/apt/sources.list"] + sorted(glob.glob("/etc/apt/sources.list.d/*")) + ["/etc/apt/preferences"] + sorted(glob.glob("/etc/apt/preferences.d/*"))
r = []
for p in paths:
    if os.path.isfile(p):
        r.append({"path": p, "content": open(p, errors="replace").read()})
print(json.dumps(r))
PYEOF"""

COMMANDS = {
    "dpkg": r"dpkg-query -W -f='${Package}\t${Version}\t${Architecture}\t${db:Status-Abbrev}\n'",
    "manual": "apt-mark showmanual",
    "holds": "apt-mark showhold",
    "uname": "uname -r",
    "proc_version": "cat /proc/version",
    "headers_present": "test -d /lib/modules/$(uname -r)/build && echo yes || echo no",
    "os_release": "cat /etc/os-release",
    "python3": "python3 --version",
    "snap": "command -v snap >/dev/null 2>&1 && snap list --color=never 2>/dev/null || true",
    "pip": PIP_PY,
    "npm": "command -v npm >/dev/null 2>&1 && npm ls -g --depth=0 --json 2>/dev/null || true",
    "gem": "command -v gem >/dev/null 2>&1 && gem list --local 2>/dev/null || true",
    "apt_files": APT_FILES_PY,
    "pyenvs": "find /opt /usr/local /home -maxdepth 5 -name pyvenv.cfg 2>/dev/null || true",
    "du": "du -x -s --block-size=1 -- /opt/* /usr/local/* /home/*/* /home/*/.[!.]* 2>/dev/null || true",
    "nft": "nft -j list ruleset 2>/dev/null || true",
    "nft_conf_sha": "sha256sum /etc/nftables.conf 2>/dev/null || true",
    "nft_enabled": "systemctl is-enabled nftables.service 2>/dev/null || true",
    "unit_states": "for u in %s; do printf '%%s\\t%%s\\n' \"$u\" \"$(systemctl is-enabled $u 2>/dev/null || true)\"; done" % " ".join(MASK_UNITS),
    "sources_present": "ls /etc/apt/sources.list /etc/apt/sources.list.d/* 2>/dev/null | wc -l",
}


def run_guest_commands(ssh, cmds, timeout=900):
    p = ssh("sudo python3 -c %s" % shlex.quote(GUEST_RUNNER_PY), stdin=json.dumps(cmds), timeout=timeout)
    require(p.returncode == 0, "MANIFEST_COLLECT_FAILED: " + (p.stderr or "")[-300:])
    line = [l for l in (p.stdout or "").splitlines() if l.startswith("{")]
    require(line, "MANIFEST_COLLECT_FAILED: no JSON from the guest")
    return json.loads(line[-1])


# ---------------------------------------------------------------- parsers (pure)
USERINFO_RE = re.compile(r"(?<=://)[^/@\s]*@")


def redact_userinfo(text):
    """`http://user:pass@host` -> `http://REDACTED@host`: apt sources are recorded without credentials."""
    return USERINFO_RE.sub("REDACTED@", text)


def parse_dpkg(out, manual=(), held=()):
    manual, held = set(manual), set(held)
    rows = []
    for l in out.splitlines():
        f = l.split("\t")
        if len(f) != 4 or f[3][1:2] != "i":   # installed only (a removed package with config files left is not in the image)
            continue
        rows.append({"name": f[0], "version": f[1], "arch": f[2], "manual": f[0] in manual, "held": f[0] in held})
    rows.sort(key=lambda r: (r["name"], r["arch"]))
    return rows


def parse_snap(out):
    rows = []
    for l in out.splitlines()[1:]:   # header line first
        f = l.split()
        if len(f) >= 4:
            rows.append({"name": f[0], "version": f[1], "revision": f[2], "channel": f[3]})
    return sorted(rows, key=lambda r: r["name"])


def parse_json_list(out):
    for l in reversed(out.strip().splitlines()):
        if l.startswith("["):
            try:
                return json.loads(l)
            except ValueError:
                return []
    return []


def parse_pip(out):
    rows = [{"name": str(r.get("name")), "version": str(r.get("version")), "location": str(r.get("location"))} for r in parse_json_list(out)]
    return sorted(rows, key=lambda r: (r["name"].lower(), r["version"]))


def parse_npm(out):
    try:
        deps = (json.loads(out) if out.strip() else {}).get("dependencies", {})
    except ValueError:
        return []
    return sorted(({"name": k, "version": str(v.get("version"))} for k, v in deps.items()), key=lambda r: r["name"])


def parse_gem(out):
    rows = []
    for l in out.splitlines():
        m = re.fullmatch(r"(\S+) \(([^)]*)\)", l.strip())
        if m:
            rows.append({"name": m.group(1), "version": m.group(2)})
    return sorted(rows, key=lambda r: r["name"])


def parse_du(out):
    rows = []
    for l in out.splitlines():
        n, _, path = l.partition("\t")
        if n.isdigit() and path.startswith("/"):
            rows.append({"path": path, "bytes": int(n)})
    return rows


def large_app_dirs(du_rows, threshold=LARGE_DIR_BYTES):
    """Directories (or files) over the threshold directly under /opt, /usr/local or a user's home that look like an application.
    A cache directory is not one (the seal cleans it)."""
    out = [r for r in du_rows if r["bytes"] > threshold and not re.search(r"/\.cache(/|$)", r["path"])]
    return sorted(out, key=lambda r: r["path"])


def sources_from_files(files):
    """[{path, content}] -> the manifest's records: path, sha256 of the REDACTED content, the redacted content."""
    rows = []
    for f in files:
        txt = redact_userinfo(f["content"])
        rows.append({"path": f["path"], "sha256": sha256_bytes(txt.encode()), "content": txt})
    return rows


def split_apt_files(files):
    src = [f for f in files if f["path"].startswith("/etc/apt/sources.list")]
    pref = [f for f in files if f["path"].startswith("/etc/apt/preferences")]
    return sources_from_files(src), sources_from_files(pref)


def summarize_inventory(rows):
    """Counts only (no paths): the files created or changed after provisioning, by kind, reason and top-level directory."""
    by_why, by_top = {}, {}
    for r in rows:
        by_why[r["why"]] = by_why.get(r["why"], 0) + 1
        top = "/" + r["path"].lstrip("/").split("/", 1)[0]
        by_top[top] = by_top.get(top, 0) + 1
    return {"entries": len(rows), "files": sum(1 for r in rows if r["type"] == "f"), "dirs": sum(1 for r in rows if r["type"] == "d"),
            "links": sum(1 for r in rows if r["type"] == "l"), "bytes": sum(r["size"] for r in rows if r["type"] == "f"),
            "by_why": dict(sorted(by_why.items())), "by_top": dict(sorted(by_top.items()))}


# ---------------------------------------------------------------- the nftables ruleset and its analysis
RULESET = """#!/usr/sbin/nft -f
# kayfabe harden-offline (docs/design/V3_PROPIMAGE_LINUX.md section 13): default drop on output and forward; loopback and
# established/related only; NO other exception. The guest is offline by design: no apt, no mirrors, no time sync.
flush ruleset
table inet kf_offline {
    chain input {
        type filter hook input priority 0; policy accept;
    }
    chain forward {
        type filter hook forward priority 0; policy drop;
    }
    chain output {
        type filter hook output priority 0; policy drop;
        oifname "lo" accept
        ct state established,related accept
    }
}
"""
VERDICTS = ("accept", "drop", "reject", "jump", "goto", "return", "queue")


def classify_rule(rule):
    """One nft JSON rule -> loopback | established | other. Counters are ignored; anything else is an exception."""
    expr = [e for e in rule.get("expr", []) if "counter" not in e]
    matches = [e["match"] for e in expr if "match" in e]
    verdicts = [k for e in expr for k in e if k in VERDICTS]
    if len(matches) != 1 or verdicts != ["accept"] or len(expr) != 2:
        return "other"
    m = matches[0]
    left = m.get("left", {})
    right = m.get("right")
    if "meta" in left and left["meta"].get("key") in ("oif", "oifname") and right in ("lo", 1) and m.get("op") == "==":
        return "loopback"
    if "ct" in left and left["ct"].get("key") == "state" and m.get("op") in ("==", "in"):
        states = set([right] if isinstance(right, str) else right if isinstance(right, list) else [])
        if states and states <= {"established", "related"}:
            return "established"
    return "other"


def analyze_nft(text):
    """`nft -j list ruleset` -> counts and the verdict inputs. ok means: an output chain and a forward chain with policy drop, and
    every rule in them is loopback or established/related (anything else is an exception and refuses the hardening)."""
    try:
        items = json.loads(text)["nftables"] if text.strip() else []
    except (ValueError, KeyError, TypeError):
        raise PkgError("HARDEN_RULESET_UNREADABLE: `nft -j list ruleset` gave no JSON")
    tables = [i["table"] for i in items if "table" in i]
    chains = [i["chain"] for i in items if "chain" in i]
    rules = [i["rule"] for i in items if "rule" in i]
    drop = {}
    for c in chains:
        if c.get("hook") in ("output", "forward") and c.get("policy") == "drop":
            drop[(c.get("family"), c.get("table"), c.get("name"))] = c["hook"]
    exceptions, kinds = [], {"loopback": 0, "established": 0, "other": 0}
    for r in rules:
        key = (r.get("family"), r.get("table"), r.get("chain"))
        if key not in drop:
            continue
        k = classify_rule(r)
        kinds[k] += 1
        if k == "other":
            exceptions.append("%s/%s: %s" % (key[1], key[2], json.dumps(r.get("expr", []), sort_keys=True)[:160]))
    hooks = sorted(set(drop.values()))
    return {"tables": len(tables), "chains": len(chains), "rules": len(rules), "drop_hooks": hooks, "rule_kinds": kinds,
            "exceptions": exceptions, "ok": hooks == ["forward", "output"] and not exceptions}


def check_ruleset(a):
    require("output" in a["drop_hooks"] and "forward" in a["drop_hooks"],
            "HARDEN_NO_DEFAULT_DROP: the ruleset has no output and forward chain with policy drop (hooks with drop: %s)" % a["drop_hooks"])
    require(not a["exceptions"], "HARDEN_EXCEPTION: rule(s) in the drop chains that are neither loopback nor established/related: %s" % a["exceptions"])
    return a


HARDEN_SCRIPT = r"""set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
# the setup VM only: apt is out after this step (the manifest records every package, nftables included)
command -v nft >/dev/null 2>&1 || apt-get install -y -qq nftables
cat > /etc/nftables.conf <<'KFEOF'
@@RULES@@
KFEOF
chmod 644 /etc/nftables.conf
nft -c -f /etc/nftables.conf
systemctl unmask nftables.service >/dev/null 2>&1 || true
systemctl enable nftables.service >/dev/null 2>&1
nft -f /etc/nftables.conf
systemctl stop apt-daily.timer apt-daily-upgrade.timer unattended-upgrades.service >/dev/null 2>&1 || true
systemctl mask @@UNITS@@ >/dev/null 2>&1
printf 'APT::Periodic::Update-Package-Lists "0";\nAPT::Periodic::Unattended-Upgrade "0";\n' > /etc/apt/apt.conf.d/20auto-upgrades
# the sources were recorded by the host before this script ran
rm -f /etc/apt/sources.list /etc/apt/sources.list.d/* /etc/apt/auth.conf /etc/apt/auth.conf.d/*
rm -rf /var/lib/apt/lists/*
echo HARDEN_OK
"""


def render_harden_script():
    s = HARDEN_SCRIPT.replace("@@RULES@@", RULESET.rstrip("\n")).replace("@@UNITS@@", " ".join(MASK_UNITS))
    require("@@" not in s, "unfilled placeholder")
    return s


OUTBOUND_PY = r"""
import json, socket, subprocess
res = {}
def tcp(h, p):
    s = socket.socket(); s.settimeout(3)
    try:
        s.connect((h, p)); return True
    except Exception:
        return False
    finally:
        s.close()
def udp_dns(h):
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.settimeout(3)
    q = b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x07example\x03com\x00\x00\x01\x00\x01"
    try:
        s.sendto(q, (h, 53)); s.recv(512); return True
    except Exception:
        return False
    finally:
        s.close()
res["tcp_1.1.1.1_443"] = tcp("1.1.1.1", 443)
res["tcp_9.9.9.9_53"] = tcp("9.9.9.9", 53)
res["udp_dns_1.1.1.1"] = udp_dns("1.1.1.1")
res["ping_1.1.1.1"] = subprocess.run("ping -c1 -W2 1.1.1.1", shell=True, capture_output=True).returncode == 0
res["loopback_ssh"] = tcp("127.0.0.1", 22)
print(json.dumps(res))
"""


def outbound_probe(ssh):
    p = ssh("python3 -c %s" % shlex.quote(OUTBOUND_PY), timeout=120)
    require(p.returncode == 0, "HARDEN_PROBE_FAILED: " + (p.stderr or "")[-200:])
    line = [l for l in (p.stdout or "").splitlines() if l.startswith("{")]
    require(line, "HARDEN_PROBE_FAILED: no JSON")
    return json.loads(line[-1])


def outbound_reachable(res):
    return sorted(k for k, v in res.items() if k != "loopback_ssh" and v)


def collect_apt_files(ssh):
    raw = run_guest_commands(ssh, {"apt_files": APT_FILES_PY}, timeout=120)
    return parse_json_list(raw["apt_files"]["out"])


def harden_offline(ssh, setup_dir, log=print):
    """Setup VM, before the seal. 1 record the apt sources on the host, 2 write the persistent ruleset and apply it, mask the apt
    timers, remove the sources and lists, 3 read the ruleset back and refuse any exception, 4 prove from inside the guest that an
    outbound attempt fails (and that it did not already fail before the rules: otherwise the proof is void). -> the record."""
    setup_dir = Path(setup_dir)
    files = collect_apt_files(ssh)
    sources, prefs = split_apt_files(files)
    rec_path = setup_dir / APT_RECORD_NAME
    if sources or not rec_path.exists():   # a second run (sources already removed) never overwrites the record with nothing
        rec_path.write_text(json.dumps({"schema": SCHEMA, "sources": sources, "preferences": prefs}, indent=1, sort_keys=True) + "\n")
    log("apt sources recorded on the host: %d file(s), %s" % (len(sources), rec_path))
    before = outbound_probe(ssh)
    p = ssh("sudo bash -s", stdin=render_harden_script(), timeout=900)
    require(p.returncode == 0 and "HARDEN_OK" in (p.stdout or ""), "HARDEN_FAILED: " + ((p.stderr or "") + (p.stdout or ""))[-400:])
    raw = run_guest_commands(ssh, {"nft": COMMANDS["nft"], "sha": COMMANDS["nft_conf_sha"], "en": COMMANDS["nft_enabled"],
                                   "units": COMMANDS["unit_states"], "src": COMMANDS["sources_present"]}, timeout=120)
    a = check_ruleset(analyze_nft(raw["nft"]["out"]))
    states = dict(l.split("\t", 1) for l in raw["units"]["out"].splitlines() if "\t" in l)
    unmasked = sorted(u for u in MASK_UNITS if states.get(u, "").strip() != "masked")
    require(not unmasked, "HARDEN_UNIT_NOT_MASKED: " + ", ".join(unmasked))
    require(raw["src"]["out"].strip() == "0", "HARDEN_SOURCES_LEFT: %s apt source file(s) remain" % raw["src"]["out"].strip())
    after = outbound_probe(ssh)
    open_ = outbound_reachable(after)
    require(not open_, "HARDEN_OUTBOUND_OPEN: still reachable after the rules: %s" % open_)
    require(after.get("loopback_ssh") is True, "HARDEN_LOOPBACK_BROKEN: 127.0.0.1:22 is not reachable inside the guest")
    note = None
    if not outbound_reachable(before):
        note = "NOTE: nothing was reachable BEFORE the rules either (the guest was already offline): the outbound check cannot show the rules did it"
    rec = {"schema": SCHEMA, "ruleset": {k: a[k] for k in ("tables", "chains", "rules", "drop_hooks", "rule_kinds")},
           "nftables_conf_sha256": raw["sha"]["out"].split()[0] if raw["sha"]["out"].split() else None, "nftables_enabled": raw["en"]["out"].strip(),
           "masked": sorted(MASK_UNITS), "apt_sources_removed": len(sources), "outbound_before": before, "outbound_after": after, "note": note}
    (setup_dir / HARDEN_RECORD_NAME).write_text(json.dumps(rec, indent=1, sort_keys=True) + "\n")
    log("ruleset: %d table(s), %d chain(s), %d rule(s); drop on %s; rules: %d loopback, %d established/related, 0 other" % (
        a["tables"], a["chains"], a["rules"], "+".join(a["drop_hooks"]), a["rule_kinds"]["loopback"], a["rule_kinds"]["established"]))
    log("outbound from inside the guest after the rules: none of %d attempts reached anything (before: %d of %d reachable); loopback ssh ok" % (
        len(after) - 1, len(outbound_reachable(before)), len(before) - 1))
    if note:
        log(note)
    return rec


def unharden(ssh):
    """A THROWAWAY clone only (the caller proves that): flush the runtime ruleset so the online logout step can reach the network.
    The persistent /etc/nftables.conf is left alone (it is the clone's overlay; the sealed image is never touched)."""
    p = ssh("sudo nft flush ruleset", timeout=60)
    require(p.returncode == 0, "UNHARDEN_FAILED: " + (p.stderr or "")[-200:])
    raw = run_guest_commands(ssh, {"nft": COMMANDS["nft"]}, timeout=60)
    a = analyze_nft(raw["nft"]["out"])
    require(not a["drop_hooks"], "UNHARDEN_STILL_DROPPING: %s" % a["drop_hooks"])
    return a


# ---------------------------------------------------------------- the manifest
def hardening_from_raw(raw):
    problems = []
    try:
        a = analyze_nft(raw["nft"]["out"])
    except PkgError as e:
        a = {"tables": 0, "chains": 0, "rules": 0, "drop_hooks": [], "rule_kinds": {}, "exceptions": [], "ok": False}
        problems.append(str(e)[:80])
    if not a["ok"]:
        problems.append("no live default-drop ruleset (drop on %s; exceptions %d)" % (a["drop_hooks"], len(a["exceptions"])))
    conf = raw["nft_conf_sha"]["out"].split()
    if not conf:
        problems.append("no persistent /etc/nftables.conf")
    enabled = raw["nft_enabled"]["out"].strip()
    if enabled != "enabled":
        problems.append("nftables.service is %r, not enabled" % enabled)
    states = dict(l.split("\t", 1) for l in raw["unit_states"]["out"].splitlines() if "\t" in l)
    unmasked = sorted(u for u in MASK_UNITS if states.get(u, "").strip() != "masked")
    if unmasked:
        problems.append("not masked: " + ", ".join(unmasked))
    n_src = raw["sources_present"]["out"].strip()
    if n_src != "0":
        problems.append("%s apt source file(s) still present" % n_src)
    return {"hardened": not problems, "problems": problems, "nftables_conf_sha256": conf[0] if conf else None, "nftables_enabled": enabled,
            "ruleset": {k: a[k] for k in ("tables", "chains", "rules", "drop_hooks", "rule_kinds")}, "unit_states": dict(sorted(states.items()))}


def build_manifest(raw, vendor, inventory, base, generated_utc, recorded_apt=None, image=None):
    """raw: the guest collector's output. -> the packages.manifest.json dict."""
    def out(k):
        return raw[k]["out"]
    live_files = parse_json_list(out("apt_files"))
    live_src, live_pref = split_apt_files(live_files)
    if live_src:
        origin, sources, prefs = "guest-live", live_src, live_pref
    elif recorded_apt and recorded_apt.get("sources"):
        origin, sources, prefs = "recorded-by-harden-offline", recorded_apt["sources"], recorded_apt.get("preferences", live_pref)
    else:
        origin, sources, prefs = "none", [], live_pref
    release = out("uname").strip()
    dpkg = parse_dpkg(out("dpkg"), out("manual").split(), out("holds").split())
    osr = dict(l.split("=", 1) for l in out("os_release").splitlines() if "=" in l)
    return {
        "schema": SCHEMA, "generated_utc": generated_utc, "image": image or {},
        "base": {k: base.get(k) for k in ("name", "release", "sha256")},
        "os": {k.lower(): osr.get(k, "").strip('"') for k in ("ID", "VERSION_ID", "VERSION_CODENAME")}, "python3": out("python3").strip(),
        "kernel": {"release": release, "headers_present": out("headers_present").strip() == "yes",
                   "proc_version": out("proc_version").strip(),
                   "packages": [{"name": r["name"], "version": r["version"]} for r in dpkg
                                if re.match(r"linux-(image|headers|modules|modules-extra)-" + re.escape(release.split("-generic")[0]), r["name"])
                                or r["name"] in ("linux-image-generic", "linux-headers-generic", "linux-generic")]},
        "dpkg": dpkg, "apt": {"sources_origin": origin, "sources": sources, "preferences": prefs, "holds": sorted(out("holds").split())},
        "snaps": parse_snap(out("snap")), "pip": parse_pip(out("pip")), "npm": parse_npm(out("npm")), "gem": parse_gem(out("gem")),
        "python_envs": sorted(l for l in out("pyenvs").splitlines() if l.strip()),
        "vendor": list(vendor), "large_dirs": large_app_dirs(parse_du(out("du"))), "inventory": inventory,
        "hardening": hardening_from_raw(raw),
        "counts": {"dpkg": len(dpkg), "manual": sum(1 for r in dpkg if r["manual"]), "snaps": len(parse_snap(out("snap"))),
                   "pip": len(parse_pip(out("pip"))), "npm": len(parse_npm(out("npm"))), "gem": len(parse_gem(out("gem"))), "vendor": len(vendor)},
    }


def collect_manifest(ssh, vendor, inventory, base, generated_utc, recorded_apt=None, image=None):
    return build_manifest(run_guest_commands(ssh, COMMANDS), vendor, inventory, base, generated_utc, recorded_apt, image)


def seal_warnings(m):
    """Lines the seal prints as WARN (never silently)."""
    w = []
    if m["large_dirs"] and not m["vendor"]:
        w.append("WARN LARGE_DIRS_WITHOUT_VENDOR_ENTRY: %s look like an application (> 1 GB) but no --vendor-entry documents one" % ", ".join(
            "%s (%.1f GB)" % (d["path"], d["bytes"] / 1e9) for d in m["large_dirs"]))
    if m["python_envs"]:
        w.append("WARN PYTHON_ENV_IN_IMAGE: %s (owner rule 3: Python environments are built at run time and copied into the clone, never sealed)" % ", ".join(m["python_envs"]))
    if not m["hardening"]["hardened"]:
        w.append("WARN NOT_HARDENED: %s (run harden-offline before the seal)" % "; ".join(m["hardening"]["problems"]))
    if m["apt"]["sources_origin"] == "none":
        w.append("WARN NO_APT_SOURCES_RECORDED: the image's packages have no recorded origin")
    return w


def validate_manifest(m):
    require(isinstance(m, dict) and m.get("schema") == SCHEMA, "PACKAGE_MANIFEST_BAD: not a %s document" % SCHEMA)
    for k in ("dpkg", "apt", "kernel", "snaps", "pip", "npm", "gem", "vendor", "large_dirs", "inventory", "hardening", "counts", "python_envs"):
        require(k in m, "PACKAGE_MANIFEST_BAD: missing " + k)
    for r in m["dpkg"]:
        require(isinstance(r.get("name"), str) and isinstance(r.get("version"), str) and r["name"] and r["version"], "PACKAGE_MANIFEST_BAD: bad dpkg row")
    require(m["counts"]["dpkg"] == len(m["dpkg"]), "PACKAGE_MANIFEST_BAD: counts.dpkg disagrees with the list")
    require(m["counts"]["vendor"] == len(m["vendor"]), "PACKAGE_MANIFEST_BAD: counts.vendor disagrees with the list")
    for v in m["vendor"]:
        require(isinstance(v, dict) and v.get("name"), "PACKAGE_MANIFEST_BAD: vendor entry without a name")
    return True


def write_manifest(m, path):
    validate_manifest(m)
    data = (json.dumps(m, indent=1, sort_keys=True) + "\n").encode()
    Path(path).write_bytes(data)
    return sha256_bytes(data)


def load_manifest(path):
    """A manifest file or a packed zip that holds one."""
    import zipfile
    p = Path(path)
    if p.suffix == ".zip":
        with zipfile.ZipFile(p) as z:
            require(MANIFEST_NAME in z.namelist(), "PACKAGE_MANIFEST_MISSING: %s has no %s" % (p.name, MANIFEST_NAME))
            m = json.loads(z.read(MANIFEST_NAME))
    else:
        m = json.loads(p.read_text())
    validate_manifest(m)
    return m


# ---------------------------------------------------------------- diff of two manifests
def _map(rows, keyf, valf):
    return {keyf(r): valf(r) for r in rows}


def diff_manifests(a, b):
    """What changed between two seals. generated_utc and the inventory counts are not part of the package diff (inventory is reported
    separately, informational)."""
    d = {}

    def delta(name, ma, mb):
        add = sorted(k for k in mb if k not in ma)
        rem = sorted(k for k in ma if k not in mb)
        chg = sorted((k, ma[k], mb[k]) for k in ma if k in mb and ma[k] != mb[k])
        if add or rem or chg:
            d[name] = {"added": [(k, mb[k]) for k in add], "removed": [(k, ma[k]) for k in rem], "changed": chg}
    key = lambda r: r["name"] + ("" if r.get("arch") in (None, "amd64", "all") else ":" + r["arch"])
    delta("dpkg", _map(a["dpkg"], key, lambda r: r["version"] + (" [held]" if r.get("held") else "") + (" [manual]" if r.get("manual") else "")),
          _map(b["dpkg"], key, lambda r: r["version"] + (" [held]" if r.get("held") else "") + (" [manual]" if r.get("manual") else "")))
    delta("apt_sources", _map(a["apt"]["sources"], lambda r: r["path"], lambda r: r["sha256"][:12]),
          _map(b["apt"]["sources"], lambda r: r["path"], lambda r: r["sha256"][:12]))
    for k in ("snaps", "pip", "npm", "gem"):
        delta(k, _map(a[k], lambda r: r["name"], lambda r: r["version"]), _map(b[k], lambda r: r["name"], lambda r: r["version"]))
    delta("vendor", _map(a["vendor"], lambda r: r["name"], lambda r: json.dumps(r, sort_keys=True)),
          _map(b["vendor"], lambda r: r["name"], lambda r: json.dumps(r, sort_keys=True)))
    delta("large_dirs", _map(a["large_dirs"], lambda r: r["path"], lambda r: "%.1f GB" % (r["bytes"] / 1e9)),
          _map(b["large_dirs"], lambda r: r["path"], lambda r: "%.1f GB" % (r["bytes"] / 1e9)))
    if a["kernel"]["release"] != b["kernel"]["release"]:
        d["kernel"] = {"changed": [("release", a["kernel"]["release"], b["kernel"]["release"])], "added": [], "removed": []}
    if a["hardening"]["hardened"] != b["hardening"]["hardened"]:
        d["hardening"] = {"changed": [("hardened", a["hardening"]["hardened"], b["hardening"]["hardened"])], "added": [], "removed": []}
    return d


def format_diff(d):
    if not d:
        return "no package-level difference\n"
    out = []
    for sect in sorted(d):
        v = d[sect]
        out.append("== %s: +%d -%d ~%d" % (sect, len(v["added"]), len(v["removed"]), len(v["changed"])))
        out += ["  + %s %s" % kv for kv in v["added"]] + ["  - %s %s" % kv for kv in v["removed"]]
        out += ["  ~ %s: %s -> %s" % t for t in v["changed"]]
    return "\n".join(out) + "\n"


# ---------------------------------------------------------------- inject: destination, tree, tar stream
def guest_roots(user):
    return ("/home/" + user, INJECT_EXTRA_ROOT)


def check_dest(dest, user):
    """-> the normalised absolute guest path. A relative dest is under the guest home; an absolute one must be STRICTLY under
    /home/<user> or /opt/kf (never the root itself, never `..`)."""
    require(isinstance(dest, str) and dest and "\x00" not in dest and "\n" not in dest, "INJECT_DEST_REFUSED: empty or unprintable destination")
    require(".." not in dest.split("/"), "INJECT_DEST_REFUSED: %r contains '..'" % dest)
    roots = guest_roots(user)
    full = posixpath.normpath(dest if dest.startswith("/") else posixpath.join(roots[0], dest))
    ok = any(full.startswith(r + "/") for r in roots)
    require(ok, "INJECT_DEST_REFUSED: %s is not under %s" % (full, " or ".join(roots)))
    return full


def _inside(real, root):
    return real == root or real.startswith(root + os.sep)


def plan_tree(src, max_bytes):
    """-> (entries [(abs path, arcname)], total regular-file bytes, counts). A symlink must be relative and stay inside the source
    (INJECT_SYMLINK_ESCAPES); sockets, fifos and devices are refused; the total is capped (INJECT_TOO_LARGE) BEFORE anything is sent."""
    src = Path(src)
    require(src.exists() or src.is_symlink(), "INJECT_SRC_MISSING: %s" % src)
    require(not src.is_symlink(), "INJECT_SYMLINK_ESCAPES: the source itself is a symlink: %s" % src)
    root = os.path.realpath(src)
    entries, total, counts = [], 0, {"files": 0, "dirs": 0, "links": 0}

    def one(p, arc):
        nonlocal total
        st = os.lstat(p)
        if os.path.islink(p):
            t = os.readlink(p)
            require(not os.path.isabs(t), "INJECT_SYMLINK_ESCAPES: %s -> %s (absolute)" % (p, t))
            real = os.path.realpath(p)
            require(_inside(real, root if src.is_dir() else os.path.dirname(root)), "INJECT_SYMLINK_ESCAPES: %s -> %s leaves the source" % (p, t))
            counts["links"] += 1
        elif os.path.isdir(p):
            counts["dirs"] += 1
        elif os.path.isfile(p):
            total += st.st_size
            counts["files"] += 1
        else:
            raise PkgError("INJECT_SPECIAL_FILE: %s is not a regular file, directory or symlink" % p)
        entries.append((str(p), arc))

    if src.is_dir():
        one(src, ".")
        for d, ds, fs in os.walk(src, followlinks=False):
            ds.sort()
            for n in sorted(ds + fs):
                p = Path(d) / n
                one(p, "./" + str(p.relative_to(src)))
    else:
        one(src, src.name)
    require(total <= max_bytes, "INJECT_TOO_LARGE: %d bytes of files exceed the cap of %d bytes (--max-gb)" % (total, max_bytes))
    return entries, total, counts


class HashingWriter:
    """The tar stream's sink: counts and hashes every byte, refuses past the cap, forwards to the ssh process."""

    def __init__(self, sink, max_bytes):
        self.sink, self.max, self.n, self.h = sink, max_bytes, 0, hashlib.sha256()

    def write(self, b):
        self.n += len(b)
        require(self.n <= self.max, "INJECT_TOO_LARGE: the tar stream passed %d bytes" % self.max)
        self.h.update(b)
        self.sink.write(b)
        return len(b)

    def flush(self):
        self.sink.flush()


def _norm(ti):
    ti.uid = ti.gid = 0
    ti.uname = ti.gname = ""
    ti.mode &= 0o1777   # no setuid/setgid bits travel
    return ti


def write_tar(entries, fileobj, max_bytes):
    w = HashingWriter(fileobj, max_bytes + (64 << 20))   # the cap on files plus headers and padding
    with tarfile.open(fileobj=w, mode="w|", format=tarfile.PAX_FORMAT) as tf:
        for path, arc in entries:
            tf.add(path, arcname=arc, recursive=False, filter=_norm)
    return {"bytes": w.n, "sha256": w.h.hexdigest()}


def remote_extract_command(dest_dir, user, replace=False):
    """The one remote command: tar over the ssh stdin into dest_dir. Modes are kept (-p); owner is the extracting user."""
    q = shlex.quote(dest_dir)
    lines = ["set -euo pipefail"]
    if dest_dir.startswith(INJECT_EXTRA_ROOT + "/"):
        lines.append("sudo install -d -o %s -g %s %s" % (user, user, INJECT_EXTRA_ROOT))
    lines += ['d=%s' % q]
    lines.append('if [ -e "$d" ]; then %s; fi' % ('rm -rf -- "$d"' if replace else 'echo INJECT_EXISTS >&2; exit 3'))
    lines += ['mkdir -p -- "$d"', 'tar -x -p --no-same-owner -C "$d"', 'echo INJECT_OK']
    return "bash -c %s" % shlex.quote("\n".join(lines))


def stream_to_guest(argv_for, entries, extract_dir, user, max_bytes, replace=False, popen=subprocess.Popen):
    """tar over ssh: no scp. -> {bytes, sha256}. The remote output must contain INJECT_OK."""
    proc = popen(argv_for(remote_extract_command(extract_dir, user, replace)), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    err = None
    try:
        res = write_tar(entries, proc.stdin, max_bytes)
    except BrokenPipeError:
        res = None
    except PkgError as e:
        err = e
    finally:
        try:
            proc.stdin.close()
        except Exception:
            pass
    out = proc.stdout.read().decode(errors="replace") if proc.stdout else ""
    rc = proc.wait()
    if err:
        raise err
    require("INJECT_EXISTS" not in out, "INJECT_DEST_EXISTS: %s already exists on the clone (use --replace)" % extract_dir)
    require(rc == 0 and res is not None and "INJECT_OK" in out, "INJECT_FAILED: rc=%s %s" % (rc, out[-300:]))
    return res


def inject_path(argv_for, src, dest, user, max_gb=DEFAULT_MAX_INJECT_GB, replace=False, popen=subprocess.Popen, log=print):
    full = check_dest(dest, user)
    require(max_gb > 0, "INJECT_BAD_CAP")
    max_bytes = int(max_gb * (1 << 30))
    entries, total, counts = plan_tree(src, max_bytes)
    extract_dir = full if Path(src).is_dir() else posixpath.dirname(full)
    if not Path(src).is_dir():
        entries = [(entries[0][0], posixpath.basename(full))]   # a file keeps the destination's name
    res = stream_to_guest(argv_for, entries, extract_dir, user, max_bytes, replace, popen)
    res.update(dest=full, files=counts["files"], dirs=counts["dirs"], links=counts["links"], file_bytes=total)
    log("inject: %d file(s), %d dir(s), %d symlink(s), %d bytes of files -> %s; tar stream %d bytes sha256 %s (host side)" % (
        counts["files"], counts["dirs"], counts["links"], total, full, res["bytes"], res["sha256"]))
    return res


# ---------------------------------------------------------------- inject --wheelhouse: copy wheels, install offline with hashes
LOCK_OPTION_RE = re.compile(r"^(--index-url|--extra-index-url|-i|--find-links|-f|--trusted-host|--no-index|--only-binary|--prefer-binary|--require-hashes)\b")
REQ_RE = re.compile(r"^([A-Za-z0-9][A-Za-z0-9._-]*)(\[[^\]]*\])?\s*==\s*([^\s;]+)\s*(;.*)?$")
WHEEL_RE = re.compile(r"^(?P<name>[^-]+)-(?P<ver>[^-]+)(-(?P<build>\d[^-]*))?-(?P<py>[^-]+)-(?P<abi>[^-]+)-(?P<plat>[^-]+)\.whl$")


def norm_name(n):
    return re.sub(r"[-_.]+", "-", n).lower()


def parse_lock(text):
    """A pip requirements lock with hashes -> {"reqs": [{name, version, hashes, marker}], "dropped": [option lines]}. Index and
    find-links options are dropped from the staged copy (the guest has no network and installs with --no-index; the wheelhouse is
    checked against the hashes instead). A requirement that cannot come from a local directory (URL, VCS, path, editable, -r) is
    LOCK_NEEDS_INDEX; an unpinned one LOCK_NOT_PINNED; one without hashes LOCK_NOT_HASHED."""
    logical, cur = [], ""
    for raw in text.splitlines():
        line = re.sub(r"(^|\s)#.*$", "", raw).rstrip()
        if line.endswith("\\"):
            cur += line[:-1] + " "
            continue
        cur += line
        if cur.strip():
            logical.append(cur.strip())
        cur = ""
    if cur.strip():
        logical.append(cur.strip())
    reqs, dropped = [], []
    for l in logical:
        if l.startswith("-"):
            require(not re.match(r"^(-r|--requirement|-c|--constraint|-e|--editable)\b", l), "LOCK_NEEDS_INDEX: %r pulls in another file or an editable path" % l[:60])
            if LOCK_OPTION_RE.match(l):
                dropped.append(l)
                continue
            raise PkgError("LOCK_UNSUPPORTED_LINE: %r" % l[:60])
        spec = l.split(" --hash=")[0].strip()
        hashes = re.findall(r"--hash=sha256:([0-9a-f]{64})", l)
        m = REQ_RE.match(spec)
        if not m:
            require(not re.search(r"@|://|^git\+|^\.|^/", spec), "LOCK_NEEDS_INDEX: %r is a direct reference; the guest has no network and no index" % spec[:60])
            raise PkgError("LOCK_NOT_PINNED: %r is not name==version" % spec[:60])
        require(hashes, "LOCK_NOT_HASHED: %s==%s has no --hash=sha256 (a hashed lock is required; see scripts/kfml/README.md)" % (m.group(1), m.group(3)))
        reqs.append({"name": m.group(1), "version": m.group(3), "hashes": sorted(set(hashes)), "marker": (m.group(4) or "").lstrip("; ").strip() or None})
    require(reqs, "LOCK_EMPTY: no requirements in the lock")
    return {"reqs": reqs, "dropped": dropped}


def staged_lock_text(text):
    """The lock as copied to the guest: the same requirements and hashes, without the index/find-links option lines."""
    keep = [l for l in text.splitlines() if not LOCK_OPTION_RE.match(l.strip())]
    return "\n".join(keep) + "\n"


def wheel_compatible(filename, py_minor, machine="x86_64", glibc=(2, 39)):
    m = WHEEL_RE.match(filename)
    if not m:
        return False
    pys, abis, plats = m["py"].split("."), m["abi"].split("."), m["plat"].split(".")

    def py_ok():
        for t in pys:
            if t == "py3":
                return True
            c = re.fullmatch(r"cp3(\d+)", t)
            if c:
                n = int(c.group(1))
                if n == py_minor and any(a in ("cp3%d" % n, "none", "abi3") for a in abis):
                    return True
                if "abi3" in abis and n <= py_minor:
                    return True
        return False

    def plat_ok():
        for p in plats:
            if p == "any" or p == "linux_" + machine:
                return True
            g = re.fullmatch(r"manylinux_(\d+)_(\d+)_" + machine, p)
            if g and (int(g.group(1)), int(g.group(2))) <= tuple(glibc):
                return True
            if p in ("manylinux2014_" + machine, "manylinux2010_" + machine, "manylinux1_" + machine):
                return True
        return False
    return py_ok() and plat_ok()


def check_wheelhouse(wheelhouse, lock, guest):
    """Every pinned requirement needs a wheel whose sha256 is one of the lock's hashes, and one of those must fit the guest
    (python minor, machine, glibc). -> {wheels: [paths to send], warnings}. Named: WHEEL_MISSING, WHEEL_HASH_MISMATCH, WHEEL_TAG_MISMATCH."""
    wh = Path(wheelhouse)
    require(wh.is_dir(), "WHEELHOUSE_MISSING: %s is not a directory" % wh)
    have = {}
    for f in sorted(wh.glob("*.whl")):
        m = WHEEL_RE.match(f.name)
        if m:
            have.setdefault((norm_name(m["name"]), m["ver"]), []).append(f)
    send, warnings = [], []
    for r in lock["reqs"]:
        cands = have.get((norm_name(r["name"]), r["version"]), [])
        label = "%s==%s" % (r["name"], r["version"])
        if not cands:
            if r["marker"]:
                warnings.append("%s has a marker (%s) and no wheel: pip will skip or fail it in the guest" % (label, r["marker"]))
                continue
            raise PkgError("WHEEL_MISSING: %s has no wheel in %s" % (label, wh))
        good = [f for f in cands if sha256_file(f) in r["hashes"]]
        require(good, "WHEEL_HASH_MISMATCH: %s: %s do(es) not match any of the lock's %d hash(es)" % (label, ", ".join(f.name for f in cands), len(r["hashes"])))
        fit = [f for f in good if wheel_compatible(f.name, guest["py_minor"], guest.get("machine", "x86_64"), tuple(guest.get("glibc", (2, 39))))]
        require(fit, "WHEEL_TAG_MISMATCH: %s: %s does not fit the guest (python 3.%d, %s, glibc %s)" % (
            label, ", ".join(f.name for f in good), guest["py_minor"], guest.get("machine"), ".".join(map(str, guest.get("glibc", ())))))
        send += good
    return {"wheels": sorted(set(send)), "warnings": warnings}


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


GUEST_INFO_PY = r"""
import json, os, platform, sys
try:
    import ensurepip, venv
    venv_ok = True
except Exception:
    venv_ok = False
g = os.confstr("CS_GNU_LIBC_VERSION") or "glibc 0.0"
print(json.dumps({"py_minor": sys.version_info[1], "py_major": sys.version_info[0], "machine": platform.machine(),
                  "glibc": [int(x) for x in g.split()[-1].split(".")[:2]], "venv_ok": venv_ok}))
"""


def guest_info(ssh):
    p = ssh("python3 -c %s" % shlex.quote(GUEST_INFO_PY), timeout=60)
    line = [l for l in (p.stdout or "").splitlines() if l.startswith("{")]
    require(p.returncode == 0 and line, "GUEST_INFO_FAILED: " + (p.stderr or "")[-200:])
    return json.loads(line[-1])


WHEEL_INSTALL = r"""set -euo pipefail
D=@@DEST@@
python3 -m venv "$D/venv"
"$D/venv/bin/python" -m pip install --no-index --no-input --disable-pip-version-check --only-binary=:all: --find-links "$D/wheelhouse" --require-hashes -r "$D/requirements.lock.txt"
echo INSTALL_OK $("$D/venv/bin/python" -m pip list --format=freeze | wc -l)
"""


def inject_wheelhouse(ssh, argv_for, wheelhouse, lock_path, dest, user, max_gb=DEFAULT_MAX_INJECT_GB, replace=False, popen=subprocess.Popen, log=print):
    """Copy a WHEELHOUSE (not a venv) and install it INSIDE the offline guest with --no-index and --require-hashes. Nothing
    Python-related is stored in the image: the target is a throwaway clone (the caller proves that). Named failures: LOCK_*,
    WHEEL_MISSING, WHEEL_HASH_MISMATCH, WHEEL_TAG_MISMATCH, GUEST_NO_VENV, PIP_INSTALL_FAILED."""
    full = check_dest(dest, user)
    lock_text = Path(lock_path).read_text()
    lock = parse_lock(lock_text)
    g = guest_info(ssh)
    require(g["py_major"] == 3, "GUEST_PYTHON: the guest's python3 is not 3.x")
    require(g["venv_ok"], "GUEST_NO_VENV: python3-venv is not in the image (apt is out in a clone: install it in the SETUP VM before the seal; it is listed in the package manifest)")
    chk = check_wheelhouse(wheelhouse, lock, g)
    for w in chk["warnings"]:
        log("WARN " + w)
    max_bytes = int(max_gb * (1 << 30))
    with tempfile.TemporaryDirectory(prefix="kf-inject-") as t:
        staged = Path(t) / "requirements.lock.txt"
        staged.write_text(staged_lock_text(lock_text))
        entries = [(str(w), "wheelhouse/" + w.name) for w in chk["wheels"]] + [(str(staged), "requirements.lock.txt")]
        total = sum(Path(p).stat().st_size for p, _ in entries)
        require(total <= max_bytes, "INJECT_TOO_LARGE: %d bytes of wheels exceed the cap of %d bytes (--max-gb)" % (total, max_bytes))
        res = stream_to_guest(argv_for, entries, full, user, max_bytes, replace, popen)
    p = ssh("bash -c %s" % shlex.quote(WHEEL_INSTALL.replace("@@DEST@@", shlex.quote(full))), timeout=3600)
    require(p.returncode == 0 and "INSTALL_OK" in (p.stdout or ""), "PIP_INSTALL_FAILED: " + ((p.stderr or "") + (p.stdout or ""))[-400:])
    res.update(dest=full, wheels=len(chk["wheels"]), requirements=len(lock["reqs"]), dropped_options=len(lock["dropped"]),
               lock_sha256=sha256_bytes(lock_text.encode()), installed=(p.stdout or "").strip().splitlines()[-1])
    log("wheelhouse: %d wheel(s) for %d pinned requirement(s), %d index/find-links option line(s) not copied; tar stream %d bytes sha256 %s" % (
        res["wheels"], res["requirements"], res["dropped_options"], res["bytes"], res["sha256"]))
    log(res["installed"] + " in " + full + "/venv (--no-index --require-hashes --only-binary)")
    return res
