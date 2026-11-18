#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Pure decisions of the GPU-PV gap probes: case table, probe output, process outcome, setter classes, Vulkan flattening (stdlib only, GPU-free tested).

STATUS: RESEARCH, 2026-11-18 (docs/design/V3_GPUPV_GAP_MATRIX.md section 3; tests: scripts/ci/test_gap_probes.py).
Nothing here runs a process or touches a device: gap_probes.py does, and calls these to decide what a result means.
"""
import fnmatch
import re

STATUSES = ("PASS", "NOT_SUPPORTED", "FAIL", "MISMATCH", "NOT_RUN")
TAGS = ("guest_only", "destructive", "terminal")
COLUMNS = ("id", "group", "kind", "prog", "timeout_s", "gpu", "tags", "env", "falsifier")
KV_MAX = 200


def load_cases(text):
    """The case table -> list of dicts. Every row has a falsifier (the result that proves `guest == host` wrong); ids are unique; tags and kinds are from the closed sets."""
    cases, seen = [], set()
    for line in text.splitlines():
        if not line.strip() or line.startswith("#") or line.startswith("id|"):
            continue
        c = line.split("|")
        if len(c) != len(COLUMNS):
            raise ValueError("gap_cases: %d columns, need %d: %r" % (len(c), len(COLUMNS), line[:80]))
        r = dict(zip(COLUMNS, c))
        if r["kind"] not in ("c", "py"):
            raise ValueError("gap_cases: kind %r" % r["kind"])
        tags = [] if r["tags"] == "-" else r["tags"].split(",")
        if any(t not in TAGS for t in tags):
            raise ValueError("gap_cases: tag in %r" % r["tags"])
        if r["gpu"] not in ("yes", "no") or not r["timeout_s"].isdigit() or int(r["timeout_s"]) < 10:
            raise ValueError("gap_cases: gpu/timeout of %s" % r["id"])
        if len(r["falsifier"].strip()) < 20:
            raise ValueError("gap_cases: %s has no falsifier" % r["id"])
        if r["id"] in seen:
            raise ValueError("gap_cases: duplicate id %s" % r["id"])
        seen.add(r["id"])
        r["tags"], r["timeout_s"] = tags, int(r["timeout_s"])
        r["env"] = {} if r["env"] == "-" else dict(kv.split("=", 1) for kv in r["env"].split(","))
        cases.append(r)
    return cases


def select(cases, side, only=None, destructive=False, terminal=False):
    """-> (run, skipped) where skipped is [(case, reason)]. A guest_only case is never run on the host side; destructive and terminal ones only when asked, and
    terminal ones last (they end the machine's session)."""
    run, skipped = [], []
    for c in cases:
        if only and not any(fnmatch.fnmatch(c["id"], g) for g in only.split(",")):
            continue
        if "guest_only" in c["tags"] and side != "guest":
            skipped.append((c, "guest_only"))
        elif "destructive" in c["tags"] and not destructive:
            skipped.append((c, "destructive_not_requested"))
        elif "terminal" in c["tags"] and not terminal:
            skipped.append((c, "terminal_not_requested"))
        else:
            run.append(c)
    run.sort(key=lambda c: ("terminal" in c["tags"], "destructive" in c["tags"]))   # stable: table order inside each class
    return run, skipped


def clean(text, limit=KV_MAX):
    return re.sub(r"[\t\r\n|]+", " ", str(text)).strip()[:limit] or "-"


def parse_probe_output(text, case):
    """-> dict(status, rc, secs, reason, kv) from the output of `gap_probe <case>`, or None when there is no GAPPROBE line for the case (a crash or a hang).
    `attr` lines (`attr 41:1`) become the keys attr.41."""
    res, kv = None, {}
    for line in text.splitlines():
        m = re.match(r"GAPKV (\S+) ([A-Za-z0-9_.]+)=(.*)$", line)
        if m and m[1] == case:
            key, val = m[2], m[3]
            if key == "attr" and ":" in val:
                key, val = "attr." + val.split(":", 1)[0], val.split(":", 1)[1]
            kv[key] = clean(val)
            continue
        m = re.match(r"GAPPROBE case=(\S+) status=(\S+) rc=(-?\d+) secs=([\d.]+) reason=(\S*)$", line)
        if m and m[1] == case and m[2] in STATUSES:
            res = {"status": m[2], "rc": int(m[3]), "secs": float(m[4]), "reason": m[5] or "-"}
    if res is None:
        return None
    res["kv"] = kv
    return res


def classify_process(rc, timed_out, parsed):
    """What a finished process means. A timeout, a crash and a missing result line are FAIL with a reason that names which: they are never NOT_RUN and never a pass."""
    if timed_out:
        return "FAIL", "timeout"
    if parsed is not None:
        return parsed["status"], parsed["reason"]
    if rc is not None and rc < 0:
        return "FAIL", "crash_signal_%d" % -rc
    if rc == 127:
        return "NOT_RUN", "cannot_execute"
    return "FAIL", "no_result_line_exit_%s" % rc


def canary_policy(status, reason, dead_count):
    """After a case: (run_canary, stop_all). A failing case triggers a canary; three dead canaries in a row stop the run (every later case would only repeat one defect)."""
    if status in ("FAIL", "MISMATCH") and reason != "-":
        return True, dead_count >= 2
    return False, False


# ------------------------------------------------------------------------------------------------------------------ nvidia-smi setters
def classify_setter(rc, output, timed_out=False):
    """OK | NOT_SUPPORTED | NO_PERMISSION | TIMEOUT | CRASH | ERROR. NOT_SUPPORTED and NO_PERMISSION are the answers a GeForce gives natively; ERROR is anything else."""
    low = output.lower()
    if timed_out:
        return "TIMEOUT"
    if rc is not None and rc < 0:
        return "CRASH"
    if rc == 0 and "not supported" not in low and "n/a" not in low:
        return "OK"
    if "not supported" in low or "unsupported" in low or "not available" in low or "is not a valid" in low and "mode" in low:
        return "NOT_SUPPORTED"
    if "insufficient permissions" in low or "must be root" in low or "permission denied" in low or "requires root" in low or "administrator" in low:
        return "NO_PERMISSION"
    return "ERROR"


SETTER_OK = ("OK", "NOT_SUPPORTED", "NO_PERMISSION")


def setters_verdict(classes, canary_ok):
    """classes: {name: class}. PASS when every setter ended in an answer a GeForce gives, and the device still answers afterwards."""
    bad = sorted(n for n, c in classes.items() if c not in SETTER_OK)
    if bad:
        return "FAIL", "setter_%s_%s" % (bad[0], classes[bad[0]].lower())
    if not canary_ok:
        return "FAIL", "device_unusable_after_setters"
    return "PASS", "-"


def host_state_changed(samples):
    """samples: list of tuples of the host fields (compute mode, persistence, power limit, application clocks) read while the guest ran setters. A guest must not change
    what the host sees: more than one distinct tuple is a leak. -> (leaked, distinct)."""
    distinct = sorted(set(samples))
    return len(distinct) > 1, distinct


# ------------------------------------------------------------------------------------------------------------------ vulkaninfo
NOISE = re.compile(r"(deviceUUID|driverUUID|pipelineCacheUUID|deviceLUID|deviceNodeMask|driverInfo|\.timestamp|vulkaninfo\.|instance\.|layers)", re.I)


def flatten_vulkan(obj, prefix=""):
    """vulkaninfo --json -> sorted `path=value` leaves, without the values that differ by construction (UUIDs, LUID, node mask, the tool's own metadata). Lists use [i]."""
    out = []
    if isinstance(obj, dict):
        for k in sorted(obj):
            out += flatten_vulkan(obj[k], prefix + ("." if prefix else "") + str(k))
    elif isinstance(obj, list):
        for i, v in enumerate(obj):
            out += flatten_vulkan(v, "%s[%d]" % (prefix, i))
    else:
        if not NOISE.search(prefix):
            out.append("%s=%s" % (prefix, obj))
    return sorted(out)


def vulkan_groups(leaves):
    """leaves -> {group: [leaf]} where group is the first two path components, so a difference is reported against `capabilities.device` and not as 4000 lines."""
    g = {}
    for l in leaves:
        path = l.split("=", 1)[0]
        g.setdefault(".".join(re.split(r"[.\[\]]+", path)[:2]), []).append(l)
    return g


def vulkan_diff(host_leaves, guest_leaves, limit=40):
    h, g = set(host_leaves), set(guest_leaves)
    only_h, only_g = sorted(h - g), sorted(g - h)
    return {"only_host": only_h[:limit], "only_guest": only_g[:limit], "n_only_host": len(only_h), "n_only_guest": len(only_g)}


# ------------------------------------------------------------------------------------------------------------------ the NVIDIA tools (compute-sanitizer, ncu, nsys)
def classify_tool(rc, output, timed_out=False):
    """OK | NOT_SUPPORTED | NO_PERMISSION | TIMEOUT | CRASH | ERROR for one run of an NVIDIA tool. The first three are answers a GPU stack may give without being broken:
    a stack that does not serve the profiler or the debugger interface says so (docs/OWNER_RULINGS.md H: capability absent); anything else is a finding."""
    low = output.lower()
    if timed_out:
        return "TIMEOUT"
    if rc is not None and rc < 0:
        return "CRASH"
    if "err_nvgpuctrperm" in low or "permission" in low and "counter" in low:
        return "NO_PERMISSION"
    if "not supported" in low or "unsupported" in low or "does not support" in low or "not available" in low or "cannot be profiled" in low:
        return "NOT_SUPPORTED"
    if rc == 0:
        return "OK"
    return "ERROR"


def tools_verdict(classes):
    """classes: {name: class}. -> (status, reason). Any TIMEOUT, CRASH or ERROR is FAIL (named). All OK is PASS. Otherwise (some tool says not supported / not permitted, none broke)
    the case is NOT_SUPPORTED, so that the host-vs-guest comparison shows a lost capability as GUEST_LOSES_SUPPORT and the known-limits file can explain it."""
    for name in sorted(classes):
        if classes[name] in ("TIMEOUT", "CRASH", "ERROR"):
            return "FAIL", "%s_%s" % (name, classes[name].lower())
    if all(c == "OK" for c in classes.values()):
        return "PASS", "-"
    return "NOT_SUPPORTED", "tool_answer_" + "_".join(sorted(n for n, c in classes.items() if c != "OK"))


def vulkan_tokens(leaves):
    """Format-independent tokens of a vulkaninfo dump, so two vulkaninfo versions with different JSON layouts can still be compared: an extension is `ext:<name>` whether it is a
    list entry (`extensionName=...`) or a key (`...extensions.<name>`); anything else is `<last identifier with its indexes>=<value>` (a limit, a feature, a property)."""
    out = set()
    for leaf in leaves:
        path, _, val = leaf.partition("=")
        if path.endswith("extensionName"):
            out.add("ext:" + val)
            continue
        m = re.search(r"\.extensions\.(VK_\w+)$", path)
        if m:
            out.add("ext:" + m[1])
            continue
        m = re.search(r"([A-Za-z_]\w*)((?:\[\d+\])*)$", path)
        if m and not path.startswith("$schema"):
            out.add("%s%s=%s" % (m[1], m[2], {"True": "1", "true": "1", "False": "0", "false": "0"}.get(val, val)))   # an older vulkaninfo writes booleans as 1 and 0
    return sorted(out)


def parse_vkprio(text):
    """Output of gap_vkprio -> dict of kv for the probe: `fam<i>.reported` (the priorities a queue family reports) and `create.<i>.<PRIORITY>` (the VkResult of vkCreateDevice with that priority),
    for the first physical device only (a second device, e.g. a software one, would mix with the first)."""
    kv, dev = {}, None
    for line in text.splitlines():
        m = re.match(r"VKPRIO device=(.*) queue_family=(\d+) flags=(0x[0-9a-f]+) count=(\d+) reported=(\S+)$", line)
        if m:
            dev = dev or m[1]
            if m[1] == dev:
                kv["fam%s.reported" % m[2]] = m[5]
                kv["fam%s.queue_flags" % m[2]] = m[3]
            continue
        m = re.match(r"VKPRIO create family=(\d+) priority=(\w+) result=(-?\d+)$", line)
        if m and ("fam%s.reported" % m[1]) in kv and ("create.%s.%s" % (m[1], m[2])) not in kv:
            kv["create.%s.%s" % (m[1], m[2])] = m[3]
    for line in text.splitlines():
        m = re.match(r"VKALLOC requested=(\d+) ok=(\d+) last_failure=(-?\d+) secs=([\d.]+) us_per_allocation=([\d.]+) memory_types=(\d+)$", line)
        if m:
            kv["alloc_requested"], kv["alloc_ok"], kv["alloc_last_failure"] = m[1], m[2], m[3]
            kv["perf.alloc_secs"], kv["perf.alloc_us_each"] = m[4], m[5]   # timings are evidence, never a difference
            break
    if dev:
        kv["info.device"] = dev
    return kv
