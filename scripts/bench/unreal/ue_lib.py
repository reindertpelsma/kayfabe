# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Pure helpers for the Unreal installed-build tooling (stdlib only, Python 3.9+, no network, no side effect on import).

STATUS: DESIGN-ONLY tooling, 2026-11-17 (GPU-free unit tests with a fake runner; nothing here was run against a real
source tree, nothing was downloaded and no build was started). Design: docs/design/V3_UNREAL_IMAGE.md.

The Unreal Engine source tree is supplied by the user and is licensed to that user; this repository neither contains nor
redistributes it. What this module computes is metadata about a build: a cache key, a manifest, disk and memory budgets,
the argument vectors of the build steps and the decision of whether to fetch, use, build or skip.

Labels in comments: ESTIMATE = a number nobody measured; CODE READ = read in the tree the user supplied, not run.
"""

import datetime as dt
import hashlib
import json
import os
import re
import shutil

KEY_SCHEMA = "kayfabe.unreal-cache-key/1"
MANIFEST_SCHEMA = "kayfabe.unreal-artifact/1"
STATE_SCHEMA = "kayfabe.unreal-build-state/1"
PIN_SCHEMA = "kayfabe.unreal-pin/1"

# ---------------------------------------------------------------------------------------------------- named classes
# Refusals and failures carry a stable class name; skips are not failures.
SKIP_NO_SOURCE_OR_CACHE = "UNREAL_NO_SOURCE_OR_CACHE"  # standalone skip: nothing to run, not a defect
FAIL_CACHE_MISS = "UNREAL_CACHE_MISS"  # strict/worker: no artifact; rebuild on the trusted machine and publish
FAIL_KEY_MISMATCH = "UNREAL_CACHE_KEY_MISMATCH"  # an artifact whose key is not the key this source + recipe produces
FAIL_WORKER_BUILD = "UNREAL_WORKER_BUILD_REFUSED"  # untrusted boxes never build
FAIL_DISK = "UNREAL_DISK_BUDGET"
FAIL_RAM = "UNREAL_RAM_BUDGET"
FAIL_SOURCE_ID = "UNREAL_SOURCE_UNIDENTIFIED"
FAIL_SOURCE_DIRTY = "UNREAL_SOURCE_DIRTY"
FAIL_SOURCE_LAYOUT = "UNREAL_SOURCE_LAYOUT"
FAIL_PREREQ = "UNREAL_PREREQ_MISSING"
FAIL_MODIFY = "UNREAL_MODIFY_SOURCE_NOT_ALLOWED"
FAIL_BUILD = "UNREAL_BUILD_FAILED"
FAIL_STATE = "UNREAL_STATE_KEY_MISMATCH"
FAIL_INTEGRITY = "UNREAL_ARTIFACT_INTEGRITY"  # a member of the artifact differs from its own manifest
FAIL_PIN = "UNREAL_PIN_MISMATCH"  # strict/worker: the artifact differs from the deployment's pin
FAIL_EXPECT = "UNREAL_EXPECT_SHA256_MISMATCH"  # the user asked for a particular sha256 and got another
FAIL_MANIFEST = "UNREAL_MANIFEST_INVALID"
FAIL_TREE = "UNREAL_TREE_INCOMPLETE"
FAIL_PATH = "UNREAL_PATH_REFUSED"
FAIL_USAGE = "UNREAL_USAGE"
FAIL_CLONE = "UNREAL_CLONE_FAILED"

MODES = ("standalone", "strict", "worker")


class UnrealError(Exception):
    """A named refusal or failure. `cls` is one of the constants above."""

    def __init__(self, cls, detail=""):
        super().__init__("%s %s" % (cls, detail) if detail else cls)
        self.cls = cls
        self.detail = detail


# ---------------------------------------------------------------------------------------------------- the recipe
# The recipe is DATA: everything that changes what the build produces is in this dict and nowhere else, so that its hash
# (`recipe_rev`) is the build-recipe revision of the cache key. Editing tests, docs or the CLI does not change the key;
# editing anything below does. RECIPE_VERSION is bumped by hand for a change that is not visible in the dict (a new
# post-processing step in `package`).
RECIPE_VERSION = 1
RECIPE = {
    "graph_script": "Engine/Build/InstalledEngineBuild.xml",  # CODE READ
    "graph_target": "Make Installed Build Linux",  # CODE READ (the node name in that script)
    "uat": "Engine/Build/BatchFiles/RunUAT.sh",
    "setup": ["./Setup.sh", "--force"],  # --force: Setup.sh adds --prompt unless --prompt or --force is given (CODE READ)
    "genproj": ["./GenerateProjectFiles.sh"],
    # BuildGraph options. HostPlatformOnly leaves the Linux AArch64 target ON when the host is Linux (CODE READ of the
    # script's default-platform properties), so it is switched off explicitly. WithDDC makes the engine's own derived-data
    # pack (Engine/DerivedDataCache/Compressed.ddp) part of the installed build.
    "set": {
        "HostPlatformOnly": "true",
        "WithLinuxArm64": "false",
        "WithDDC": "true",
        "WithClient": "false",
        "WithServer": "false",
        "CompileDatasmithPlugins": "false",
        "GameConfigurations": "Development",
    },
    "target": {"platform": "Linux", "arch": "x86_64"},
    "archive": {"format": "tar", "compression": "zstd", "level": 3},
    "required_in_tree": [
        "Engine/Build/InstalledBuild.txt",  # CODE READ: written by the script's FinalizeInstalledBuild command
        "Engine/Build/Build.version",
        "Engine/DerivedDataCache/Compressed.ddp",  # CODE READ: tagged by the script's DDC node
    ],
    "expected_in_tree": [  # missing = WARN only (names not verified against a build)
        "Engine/Binaries/Linux/UnrealEditor",
        "Engine/Binaries/Linux/UnrealEditor-Cmd",
        "Engine/Binaries/Linux/ShaderCompileWorker",
        "Engine/Build/BatchFiles/RunUAT.sh",
    ],
}

RE_HEX40 = re.compile(r"^[0-9a-f]{40}$")
RE_HEX64 = re.compile(r"^[0-9a-f]{64}$")
RE_UTC = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")


def canonical(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def sha256_bytes(raw):
    return hashlib.sha256(raw).hexdigest()


def sha256_file(path, chunk=1 << 20):
    h = hashlib.sha256()
    size = 0
    with open(path, "rb") as f:
        while True:
            b = f.read(chunk)
            if not b:
                break
            h.update(b)
            size += len(b)
    return h.hexdigest(), size


def recipe_rev(recipe=None):
    """`r<version>-<12 hex of the recipe's canonical hash>`."""
    r = RECIPE if recipe is None else recipe
    return "r%d-%s" % (RECIPE_VERSION, sha256_bytes(canonical(r).encode())[:12])


def utc_now():
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def parse_utc(text):
    if not isinstance(text, str) or not RE_UTC.match(text):
        raise UnrealError(FAIL_MANIFEST, "not a UTC time: %r" % (text,))
    return dt.datetime.strptime(text, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=dt.timezone.utc)


# ---------------------------------------------------------------------------------------------------- the runner
class Runner:
    """The one place a process is started. The fake used by the tests has the same two methods."""

    def capture(self, argv, cwd=None, env=None, timeout=300):
        import subprocess

        try:
            p = subprocess.run(argv, cwd=cwd, env=env, capture_output=True, timeout=timeout)
        except (OSError, subprocess.TimeoutExpired):
            return 127, b""
        return p.returncode, p.stdout

    def run(self, argv, cwd, env, log_path, timeout):
        """Run to completion with stdout and stderr appended to `log_path` (never held in memory). A start marker and an
        exit line are written to the log, so a killed job and a running job can be told apart. Returns the exit code
        (124 for a timeout)."""
        import subprocess
        import time

        with open(log_path, "ab") as log:
            log.write(("KF_UNREAL_START %s argv0=%s\n" % (utc_now(), os.path.basename(argv[0]))).encode())
            log.flush()
            t0 = time.time()
            try:
                p = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
            except OSError as e:
                log.write(("KF_UNREAL_EXIT rc=127 error=%s\n" % e.__class__.__name__).encode())
                return 127
            try:
                rc = p.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                p.kill()
                p.wait()
                rc = 124
            log.write(("KF_UNREAL_EXIT rc=%d secs=%d\n" % (rc, time.time() - t0)).encode())
            return rc


# The environment of every build step: an allow-list. No credential reaches Setup.sh, the compiler or BuildGraph. Only
# the clone step may carry a GitHub token, and only if the user exported one (the gh CLI's own login needs none).
ENV_ALLOW = ("PATH", "HOME", "LANG", "LC_ALL", "TMPDIR", "USER", "LOGNAME", "SHELL")
ENV_FORCED = {
    "GIT_TERMINAL_PROMPT": "0",  # never prompt for credentials
    "GH_PROMPT_DISABLED": "1",
    "DOTNET_CLI_TELEMETRY_OPTOUT": "1",
    "DOTNET_NOLOGO": "1",
}
ENV_CLONE_EXTRA = ("GH_TOKEN", "GITHUB_TOKEN", "GH_HOST", "GH_CONFIG_DIR", "XDG_CONFIG_HOME")


def build_env(environ, clone=False):
    keep = ENV_ALLOW + (ENV_CLONE_EXTRA if clone else ())
    env = {k: environ[k] for k in keep if k in environ}
    env.update(ENV_FORCED)
    return env


# ---------------------------------------------------------------------------------------------------- the source tree
def _read_json(path, what):
    try:
        with open(path, "rb") as f:
            raw = f.read(1 << 20)
        # The engine's own JSON files may carry a BOM or tabs; json handles tabs.
        return json.loads(raw.decode("utf-8-sig"))
    except (OSError, ValueError, UnicodeError):
        raise UnrealError(FAIL_SOURCE_LAYOUT, "cannot read %s" % what) from None


def read_build_version(src):
    d = _read_json(os.path.join(src, "Engine/Build/Build.version"), "Engine/Build/Build.version")
    try:
        v = {
            "major": int(d["MajorVersion"]),
            "minor": int(d["MinorVersion"]),
            "patch": int(d["PatchVersion"]),
            "branch": str(d.get("BranchName", "")),
            "compatible_changelist": int(d.get("CompatibleChangelist", 0)),
            "is_promoted": int(d.get("IsPromotedBuild", 0)),
        }
    except (KeyError, TypeError, ValueError):
        raise UnrealError(FAIL_SOURCE_LAYOUT, "Build.version has no version numbers") from None
    v["version"] = "%d.%d.%d" % (v["major"], v["minor"], v["patch"])
    return v


def read_toolchain(src):
    """The Linux toolchain bundle the source asks Setup.sh to fetch (its SDK version string; the clang version is part of
    that string). The key uses THIS value, which can be read before anything is built; the clang the build really ran is
    recorded in the manifest as an observation."""
    d = _read_json(os.path.join(src, "Engine/Config/Linux/Linux_SDK.json"), "Engine/Config/Linux/Linux_SDK.json")
    v = d.get("MainVersion")
    if not isinstance(v, str) or not re.fullmatch(r"[A-Za-z0-9_.+-]{1,120}", v):
        raise UnrealError(FAIL_SOURCE_LAYOUT, "Linux_SDK.json has no usable MainVersion")
    return {"sdk_version": v}


def deps_manifest_digest(src):
    p = os.path.join(src, "Engine/Build/Commit.gitdeps.xml")
    if not os.path.isfile(p):
        return None
    return sha256_file(p)[0]


def check_layout(src):
    for rel in ("Setup.sh", "GenerateProjectFiles.sh", RECIPE["graph_script"], "Engine/Build/Build.version", "Engine/Config/Linux/Linux_SDK.json"):
        if not os.path.isfile(os.path.join(src, rel)):
            raise UnrealError(FAIL_SOURCE_LAYOUT, "not an Unreal source root: %s is missing" % rel)


def source_identity(runner, src, allow_dirty=False, source_id=None):
    """Who this source tree is. A git checkout: its commit (and, if the tracked files differ from it, a digest of the
    difference; refused unless `allow_dirty`). A tree with no git history (an unpacked archive): only an explicit
    `source_id` identifies it. Returns {"kind", "id", "dirty_digest"}."""
    env = build_env(os.environ)
    rc, out = runner.capture(["git", "-C", src, "rev-parse", "--verify", "HEAD"], env=env)
    commit = out.decode("ascii", "replace").strip().lower() if rc == 0 else ""
    if RE_HEX40.match(commit):
        rc, st = runner.capture(["git", "-C", src, "status", "--porcelain=v1", "--untracked-files=no", "-z"], env=env)
        if rc != 0:
            raise UnrealError(FAIL_SOURCE_ID, "git status failed")
        if not st.strip(b"\0"):
            return {"kind": "git", "id": commit, "dirty_digest": None}
        if not allow_dirty:
            raise UnrealError(FAIL_SOURCE_DIRTY, "tracked files differ from %s; commit or stash, or pass --allow-dirty" % commit[:12])
        rc, diff = runner.capture(["git", "-C", src, "diff", "HEAD", "--binary"], env=env, timeout=1800)
        if rc != 0:
            raise UnrealError(FAIL_SOURCE_ID, "git diff failed")
        return {"kind": "git", "id": commit, "dirty_digest": sha256_bytes(st + b"\0\0" + diff)}
    if not source_id or not re.fullmatch(r"[A-Za-z0-9_.:+-]{8,128}", source_id):
        raise UnrealError(FAIL_SOURCE_ID, "no git history here: give --source-id (8-128 of [A-Za-z0-9_.:+-], e.g. the commit you unpacked)")
    return {"kind": "declared", "id": source_id, "dirty_digest": None}


# ---------------------------------------------------------------------------------------------------- the cache key
def key_inputs(identity, version, toolchain, deps_digest, recipe=None):
    r = RECIPE if recipe is None else recipe
    return {
        "schema": KEY_SCHEMA,
        "source": {"kind": identity["kind"], "id": identity["id"], "dirty_digest": identity["dirty_digest"]},
        "deps_manifest_sha256": deps_digest,
        "engine": {k: version[k] for k in ("major", "minor", "branch", "compatible_changelist", "is_promoted", "patch")},
        "recipe_rev": recipe_rev(r),
        "toolchain": dict(toolchain),
        "target": dict(r["target"]),
        "flags": {"graph_target": r["graph_target"], "set": dict(r["set"])},
    }


def cache_key(inputs):
    return sha256_bytes(canonical(inputs).encode())


def compute_key(runner, src, allow_dirty=False, source_id=None):
    check_layout(src)
    ident = source_identity(runner, src, allow_dirty, source_id)
    version = read_build_version(src)
    inputs = key_inputs(ident, version, read_toolchain(src), deps_manifest_digest(src))
    return {"key": cache_key(inputs), "inputs": inputs, "version": version["version"], "identity": ident}


def object_name(version, key, prefix="linux-proprietary", name="artifact.tar.zst"):
    """`<prefix>/unreal-<version>-<12 hex of the key>/<name>`: a name derived from the key, so a different source or recipe
    can never overwrite another build's object."""
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version) or not RE_HEX64.match(key):
        raise UnrealError(FAIL_USAGE, "bad version or key")
    if not re.fullmatch(r"[A-Za-z0-9._-]+(/[A-Za-z0-9._-]+)*", prefix) or ".." in prefix.split("/"):
        raise UnrealError(FAIL_USAGE, "bad object prefix")
    return "%s/unreal-%s-%s/%s" % (prefix, version, key[:12], name)


def build_id_for(key):
    """A GUID-shaped build id derived from the key, passed as the script's BuildIdOverride so the build's own random id does
    not make two builds of one key differ. UNVERIFIED that the script accepts exactly this shape."""
    h = key[:32]
    return "%s-%s-%s-%s-%s" % (h[:8], h[8:12], h[12:16], h[16:20], h[20:32])


# ---------------------------------------------------------------------------------------------------- budgets
GB = 10**9
# ESTIMATE (nobody has built this tree here; Epic documents roughly 200+ GB for a source build). Per phase, the ADDITIONAL space
# the phase needs on each location. A resumed run checks again at the start of each remaining phase.
PHASE_BUDGET_GB = {
    "clone": {"source": 40},  # shallow clone: objects plus working tree; a full-history clone of a large fork needs more
    "setup": {"source": 25},  # the dependency set Setup.sh downloads, plus the toolchain archive and its unpack
    "genproj": {"source": 2},
    "buildgraph": {"source": 110, "installed": 45},  # intermediates and the DDC staging copy; the installed output
    "package": {"out": 22},  # the compressed archive (streamed; no second uncompressed copy)
}
PHASE_HOURS = {"clone": (0.2, 1.0), "setup": (0.2, 1.5), "genproj": (0.05, 0.2), "buildgraph": (3.0, 7.0), "package": (0.3, 0.8)}  # ESTIMATE
MIN_RAM_GB = 24  # below this the editor link is likely to run out of memory (ESTIMATE); 32 or more is recommended
RAM_PER_ACTION_GB = 1.5  # ESTIMATE for a compile action of the heaviest translation units
DEFAULT_RESERVE_GB = 10
PHASE_ORDER = ("clone", "setup", "genproj", "buildgraph", "package")


def phases_for(clone, package):
    return [p for p in PHASE_ORDER if (p != "clone" or clone) and (p != "package" or package)]


def location_paths(source, installed, out):
    return {"source": source, "installed": installed, "out": out}


def _existing_parent(path):
    p = os.path.abspath(path)
    while not os.path.exists(p):
        parent = os.path.dirname(p)
        if parent == p:
            break
        p = parent
    return p


def free_bytes(path):
    return shutil.disk_usage(_existing_parent(path)).free


def disk_need(phase, locations, reserve_gb=DEFAULT_RESERVE_GB, dev_of=None):
    """Group the phase's per-location budgets by filesystem (st_dev of the nearest existing parent): two locations on one
    filesystem add up. Returns {device: {"need": bytes, "locations": [names], "path": a path on it}}."""
    dev_of = dev_of or (lambda p: os.stat(_existing_parent(p)).st_dev)
    groups = {}
    for loc, gb in PHASE_BUDGET_GB[phase].items():
        path = locations[loc]
        g = groups.setdefault(dev_of(path), {"need": reserve_gb * GB, "locations": [], "path": path})
        g["need"] += gb * GB
        g["locations"].append(loc)
    return groups


def check_disk(phase, locations, reserve_gb=DEFAULT_RESERVE_GB, free_fn=free_bytes, dev_of=None):
    """Refuse (UNREAL_DISK_BUDGET) when any filesystem has less free space than the phase's budget plus the reserve. There is
    no override: a build that runs out of disk half way costs hours."""
    for dev, g in sorted(disk_need(phase, locations, reserve_gb, dev_of).items(), key=lambda kv: str(kv[0])):
        free = free_fn(g["path"])
        if free < g["need"]:
            raise UnrealError(
                FAIL_DISK,
                "phase %s needs %.0f GB free (budget plus %d GB reserve) for %s, %.0f GB is free at %s"
                % (phase, g["need"] / GB, reserve_gb, "+".join(g["locations"]), free / GB, g["path"]),
            )


def check_ram(total_gb):
    if total_gb < MIN_RAM_GB:
        raise UnrealError(FAIL_RAM, "%.0f GB of RAM; the build needs at least %d GB (32 recommended)" % (total_gb, MIN_RAM_GB))


def compute_jobs(cpus, ram_gb, requested=None):
    """Parallel compile actions: bounded by CPUs and by memory. `requested` can only lower it."""
    by_ram = max(1, int((ram_gb - 4) // RAM_PER_ACTION_GB))
    n = max(1, min(int(cpus), by_ram))
    if requested is not None:
        n = max(1, min(n, int(requested)))
    return n


def plan_text(phases):
    lo = sum(PHASE_HOURS[p][0] for p in phases)
    hi = sum(PHASE_HOURS[p][1] for p in phases)
    lines = []
    total = 0
    for p in phases:
        gb = sum(PHASE_BUDGET_GB[p].values())
        total += gb
        lines.append("  %-10s +%3d GB  %.1f-%.1f h  (ESTIMATE)" % (p, gb, PHASE_HOURS[p][0], PHASE_HOURS[p][1]))
    lines.append("  %-10s +%3d GB  %.1f-%.1f h  (ESTIMATE; before any reclaim)" % ("total", total, lo, hi))
    return "\n".join(lines)


# ---------------------------------------------------------------------------------------------------- build steps
def buildgraph_argv(src, installed_dir, key, jobs, recipe=None):
    """The BuildGraph invocation. -clean is never passed: it would throw away the work a resumed run is entitled to keep."""
    r = RECIPE if recipe is None else recipe
    argv = [os.path.join(src, r["uat"]), "BuildGraph", "-target=" + r["graph_target"], "-script=" + r["graph_script"]]
    for k, v in sorted(r["set"].items()):
        argv.append("-set:%s=%s" % (k, v))
    argv.append("-set:BuiltDirectory=" + installed_dir)
    argv.append("-set:BuildIdOverride=" + build_id_for(key))
    argv.append("-set:ExtraCompileArgs=-MaxParallelActions=%d" % jobs)
    return argv


def installed_tree_dir(installed_dir):
    """Where the script puts the Linux installed tree under BuiltDirectory (CODE READ: `$(LocalInstalledDirLinux)` is
    `$(BuiltDirectory)/Linux`)."""
    return os.path.join(installed_dir, "Linux")


def verify_tree(tree):
    """Returns (problems, warnings). A required file missing is a problem."""
    problems = [p for p in RECIPE["required_in_tree"] if not os.path.isfile(os.path.join(tree, p))]
    warnings = [p for p in RECIPE["expected_in_tree"] if not os.path.isfile(os.path.join(tree, p))]
    return problems, warnings


def tree_stats(tree):
    files = size = 0
    for root, _dirs, names in os.walk(tree):
        for n in names:
            p = os.path.join(root, n)
            try:
                if not os.path.islink(p):
                    size += os.path.getsize(p)
                files += 1
            except OSError:
                pass
    return {"files": files, "bytes": size}


def parse_ldd_missing(text):
    """sonames that `ldd` reports as not found, from the text of one or more ldd runs."""
    return sorted(set(re.findall(r"^\s*(\S+) => not found\s*$", text, re.M)))


def tar_zstd_argv(tree, level, threads=0):
    """The two processes of the streamed archive: sorted names, fixed owner, fixed mtime (the commit or build time, passed
    by the caller), so two archives of one tree differ only if file contents differ. Bit-identity of the compressed output
    is UNVERIFIED."""
    tar = ["tar", "--sort=name", "--owner=0", "--group=0", "--numeric-owner", "--format=gnu", "-C", tree, "-cf", "-", "."]
    zstd = ["zstd", "-q", "-%d" % level, "-T%d" % threads, "--no-progress"]
    return tar, zstd


# ---------------------------------------------------------------------------------------------------- state (resume)
class State:
    """work/state.json: which phases finished, for which key. Atomic writes. A different key refuses to resume."""

    def __init__(self, path, key):
        self.path = path
        self.key = key
        self.doc = {"schema": STATE_SCHEMA, "key": key, "phases": {}}
        if os.path.exists(path):
            try:
                with open(path, "rb") as f:
                    d = json.loads(f.read(1 << 20).decode())
            except (OSError, ValueError, UnicodeError):
                raise UnrealError(FAIL_STATE, "state.json is unreadable; remove it or pass --restart") from None
            if d.get("schema") != STATE_SCHEMA or not isinstance(d.get("phases"), dict):
                raise UnrealError(FAIL_STATE, "state.json is not a build state; remove it or pass --restart")
            if d.get("key") != key:
                raise UnrealError(FAIL_STATE, "state.json belongs to another key (%s); pass --restart or use another --work" % str(d.get("key"))[:12])
            self.doc = d

    def done(self, phase):
        return bool(self.doc["phases"].get(phase, {}).get("done"))

    def mark(self, phase, rc, secs=None):
        self.doc["phases"][phase] = {"done": rc == 0, "rc": rc, "utc": utc_now(), "secs": secs}
        self.save()

    def save(self):
        tmp = self.path + ".tmp"
        with open(tmp, "w") as f:
            json.dump(self.doc, f, sort_keys=True, indent=1)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, self.path)


# ---------------------------------------------------------------------------------------------------- the manifest
_MANIFEST_KEYS = {
    "schema", "key", "key_inputs", "artifact", "engine", "source", "recipe", "toolchain", "built_utc", "created_utc",
    "loads_software", "host_libs", "builder", "reproducibility", "notes",
}
_ARTIFACT_KEYS = {"name", "size", "sha256", "compression", "level", "uncompressed_bytes", "files"}

REPRODUCIBILITY = "reproducible modulo timestamps; bit-identity of two builds is UNVERIFIED"


def make_manifest(comp, artifact, stats, clang_observed, host_libs, builder, kayfabe_rev, built_utc=None):
    t = built_utc or utc_now()
    ident = comp["identity"]
    return {
        "schema": MANIFEST_SCHEMA,
        "key": comp["key"],
        "key_inputs": comp["inputs"],
        "artifact": {
            "name": artifact["name"], "size": artifact["size"], "sha256": artifact["sha256"],
            "compression": "zstd", "level": RECIPE["archive"]["level"],
            "uncompressed_bytes": stats["bytes"], "files": stats["files"],
        },
        "engine": {"version": comp["version"]},
        "source": {"kind": ident["kind"], "id": ident["id"], "dirty": ident["dirty_digest"] is not None},
        "recipe": {"rev": recipe_rev(), "version": RECIPE_VERSION, "kayfabe_rev": kayfabe_rev},
        "toolchain": {"sdk_version": comp["inputs"]["toolchain"]["sdk_version"], "clang_observed": clang_observed},
        "built_utc": t,
        "created_utc": t,
        "loads_software": True,
        "host_libs": sorted(set(host_libs)),
        "builder": dict(builder),
        "reproducibility": REPRODUCIBILITY,
        "notes": "an installed build of an Unreal Engine source tree supplied by the user; not redistributable by this project",
    }


def validate_manifest(doc):
    """Strict. Raises UnrealError(UNREAL_MANIFEST_INVALID). The key is RECOMPUTED from key_inputs and must match."""
    def bad(msg):
        raise UnrealError(FAIL_MANIFEST, msg)

    if not isinstance(doc, dict) or doc.get("schema") != MANIFEST_SCHEMA:
        bad("not an unreal artifact manifest")
    extra = set(doc) - _MANIFEST_KEYS
    if extra:
        bad("unknown keys %s" % sorted(extra))
    if not isinstance(doc.get("key_inputs"), dict) or doc["key_inputs"].get("schema") != KEY_SCHEMA:
        bad("key_inputs missing or of another schema")
    if not (isinstance(doc.get("key"), str) and RE_HEX64.match(doc["key"])):
        bad("key is not 64 hex")
    if cache_key(doc["key_inputs"]) != doc["key"]:
        bad("key does not equal the hash of key_inputs")
    a = doc.get("artifact")
    if not isinstance(a, dict) or set(a) - _ARTIFACT_KEYS or not {"name", "size", "sha256"} <= set(a):
        bad("artifact member malformed")
    if not (isinstance(a["name"], str) and re.fullmatch(r"[A-Za-z0-9._-]{1,100}", a["name"])):
        bad("artifact name")
    if type(a["size"]) is not int or a["size"] <= 0:
        bad("artifact size")
    if not (isinstance(a["sha256"], str) and RE_HEX64.match(a["sha256"])):
        bad("artifact sha256")
    if doc.get("loads_software") is not True:
        bad("loads_software must be true (the clock rule counts the artifact as loaded software)")
    parse_utc(doc.get("created_utc"))
    parse_utc(doc.get("built_utc"))
    if not isinstance(doc.get("host_libs"), list) or not all(isinstance(x, str) for x in doc["host_libs"]):
        bad("host_libs")
    return doc


def load_manifest(path):
    try:
        with open(path, "rb") as f:
            raw = f.read((1 << 20) + 1)
        if len(raw) > 1 << 20:
            raise ValueError
        doc = json.loads(raw.decode())
    except (OSError, ValueError, UnicodeError):
        raise UnrealError(FAIL_MANIFEST, "unreadable manifest") from None
    return validate_manifest(doc)


def check_member(path, manifest):
    """Integrity: the file the user holds is the one its own manifest describes (size and sha256). Applies in every mode.
    Returns the computed (sha256, size) for the evidence record."""
    sha, size = sha256_file(path)
    a = manifest["artifact"]
    if size != a["size"] or sha != a["sha256"]:
        raise UnrealError(FAIL_INTEGRITY, "%s: size %d sha256 %s differ from its manifest (size %d sha256 %s)" % (os.path.basename(path), size, sha[:12], a["size"], a["sha256"][:12]))
    return sha, size


# ---------------------------------------------------------------------------------------------------- pins and upload
def pin_entry(manifest, prefix="linux-proprietary"):
    """The deployment's own config entry for this artifact, in the shape of the `objects` list of ci/vast-broker/propimages.json
    plus the fields the clock rule reads (`loads_software`, `created_utc`). It is DEPLOYMENT data: only strict and worker
    runs enforce it; a standalone run records what it computed and enforces nothing from it."""
    a = manifest["artifact"]
    version = manifest["engine"]["version"]
    obj = object_name(version, manifest["key"], prefix, a["name"])
    return {
        "id": "unreal-linux",
        "role": "artifact",
        "ref": "unreal-%s-%s.tar.zst" % (version, manifest["key"][:12]),
        "object": obj,
        "size": a["size"],
        "sha256": a["sha256"],
        "key": manifest["key"],
        "loads_software": True,
        "created_utc": manifest["created_utc"],
    }


def manifest_pin_entry(manifest, manifest_path, prefix="linux-proprietary"):
    sha, size = sha256_file(manifest_path)
    return {
        "id": "unreal-linux-manifest",
        "role": "manifest",
        "object": object_name(manifest["engine"]["version"], manifest["key"], prefix, "manifest.json"),
        "size": size,
        "sha256": sha,
    }


def download_manifest(pin, manifest_pin=None):
    """A `kayfabe.download-manifest/1` document (scripts/ci/box_downloader.py) that fetches the artifact through a presigned URL
    file: the plain-S3 route. `ref` names the entry of the --urls file."""
    items = [{"name": pin["ref"], "dest": "dl/" + pin["ref"], "sha256": pin["sha256"], "size": pin["size"], "needed_by": "unreal",
              "priority": 0, "sources": [{"kind": "signed", "ref": pin["ref"]}]}]
    return {"schema": "kayfabe.download-manifest/1", "stages": ["unreal"], "items": items}


def s3_upload_plan(endpoint, bucket, files):
    """Command lines for ANY S3-compatible store (R2, MinIO, Ceph, AWS). Nothing here is R2-specific: a plain PutObject through
    the `aws` CLI or `rclone`. Credentials come from the standard AWS_* environment variables (or rclone's own config), never
    from an argument. Returns a list of argv lists (not executed here)."""
    if not re.fullmatch(r"https?://[A-Za-z0-9.:\[\]-]+(/[A-Za-z0-9._~/-]*)?", endpoint or ""):
        raise UnrealError(FAIL_USAGE, "bad endpoint")
    if not re.fullmatch(r"[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]", bucket or ""):
        raise UnrealError(FAIL_USAGE, "bad bucket")
    plans = []
    for local, obj in files:
        plans.append(["aws", "s3", "cp", "--only-show-errors", "--endpoint-url", endpoint, local, "s3://%s/%s" % (bucket, obj)])
    return plans


# ---------------------------------------------------------------------------------------------------- rows
ROW_KEYS = {"id", "title", "lane", "phase", "needs", "cmd", "time_box_s", "verify", "gpu_assert", "names", "notes"}
VERIFY_KINDS = {"log_regex", "automation_report", "image_nonflat", "image_parity", "vk_caps_parity", "exit_zero_only_is_never_enough"}
GPU_ASSERT = {"mem", "util", "none"}
NAME_STATUS = {"VERIFIED_IN_SOURCE", "UNVERIFIED", "TOOL"}


def validate_rows(doc):
    """Returns a list of problems. Every row starts in the discovery lane; every row names its host verification; a row without
    a GPU-use assertion says why in `notes`; exit status zero alone is never a verification."""
    probs = []
    if doc.get("schema") != "kayfabe.unreal-rows/1":
        probs.append("schema")
    seen = set()
    for r in doc.get("rows", []):
        rid = r.get("id")
        if set(r) - ROW_KEYS or not {"id", "lane", "cmd", "verify", "gpu_assert", "names"} <= set(r):
            probs.append("%s: keys" % rid)
            continue
        if not (isinstance(rid, str) and re.fullmatch(r"ue_[a-z0-9_]{2,40}", rid)) or rid in seen:
            probs.append("%s: id" % rid)
        seen.add(rid)
        if r["lane"] != "discovery":
            probs.append("%s: new rows start in discovery" % rid)
        if not r["verify"] or not set(r["verify"]) <= VERIFY_KINDS - {"exit_zero_only_is_never_enough"}:
            probs.append("%s: verify" % rid)
        if r["gpu_assert"] not in GPU_ASSERT:
            probs.append("%s: gpu_assert" % rid)
        if r["gpu_assert"] == "none" and not r.get("notes"):
            probs.append("%s: no GPU assertion needs a reason" % rid)
        if not isinstance(r["cmd"], list) or not r["cmd"] or not all(isinstance(x, str) for x in r["cmd"]):
            probs.append("%s: cmd" % rid)
        if not (isinstance(r["names"], dict) and r["names"] and set(r["names"].values()) <= NAME_STATUS):
            probs.append("%s: names" % rid)
    return probs
