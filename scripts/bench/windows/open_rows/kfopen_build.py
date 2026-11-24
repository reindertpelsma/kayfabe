# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Toolchain, pinned sources and builds of the open Windows rows (STATUS: DESIGN-ONLY, 2026-11-17; the decisions are GPU-free and tested with
fakes, the commands themselves never ran on a Windows guest).

Every step builds argument lists for kfopen_common.Runner (no shell). Every download goes through kfopen_pins anchors: an upstream checksum fetched
at run time, a vendor code signature, the PyPI digest enforced by pip, or the nuget.org repository signature. Sources are clones of a pinned upstream
commit, checked with `git rev-parse HEAD`; a mismatch is a refusal. Standalone defaults to trusting git only; every lift is explicit and recorded
(kfopen_common.Trust). Nothing is fetched from a private object, an account or a secret, and no cache is read: a prebuilt tree is only ever one the
user passes with --prebuilt (trusted, hash recorded).
"""
import os
import urllib.request
import zipfile

import kfopen_pins as P
from kfopen_common import HERE, OpenRowError, Runner, Trust, canonical, load_json, say, sha256_file, sha256_text, utc, write_json

ROW_NEEDS = {"cts-gl": ("git", "python", "msvc"), "cts-vk": ("git", "python", "lxml", "msvc"), "gl-soak": ("msvc",),
             "dx-window": ("git", "msvc", "nuget"), "dx-console": ("git", "msvc", "nuget"), "not-run": ()}
DEFAULT_ROOT = r"C:\kfopen"
VSWHERE = r"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
MAX_DOWNLOAD = 8 << 30


class Fetcher:
    """https only; sha256 computed while writing. The tests replace it."""

    def _open(self, url, accept=None):
        if not url.startswith("https://"):
            raise OpenRowError("FETCH_REFUSED only https URLs are fetched: %s" % url)
        req = urllib.request.Request(url, headers={"User-Agent": "kayfabe-open-rows", "Accept": accept or "*/*"})
        return urllib.request.urlopen(req, timeout=120)

    def get_text(self, url, accept=None):
        with self._open(url, accept) as r:
            return r.read(32 << 20).decode("utf-8", "replace")

    def download(self, url, dest):
        import hashlib
        h, n = hashlib.sha256(), 0
        os.makedirs(os.path.dirname(os.path.abspath(dest)), exist_ok=True)
        with self._open(url) as r, open(dest, "wb") as f:
            for block in iter(lambda: r.read(1 << 20), b""):
                n += len(block)
                if n > MAX_DOWNLOAD:
                    raise OpenRowError("FETCH_TOO_BIG %s" % url)
                h.update(block)
                f.write(block)
        return h.hexdigest()


class Ctx:
    def __init__(self, pins, root=None, runner=None, fetcher=None, trust=None, log=say, jobs=None):
        self.pins, self.root = pins, root or os.environ.get("KFOPEN_ROOT") or DEFAULT_ROOT
        self.runner, self.fetcher, self.trust = runner or Runner(), fetcher or Fetcher(), trust or Trust()
        self.log, self.jobs = log, jobs or min(8, os.cpu_count() or 2)
        self.receipt = {"schema": 1, "date": utc(), "trust": self.trust.record(), "sources": {}, "tools": {}, "builds": {}}
        self.vs = None

    def p(self, *parts):
        return os.path.join(self.root, *parts)

    def env(self):
        """PATH with the tools this module installed first; nothing else is changed."""
        env = dict(os.environ)
        extra = [d for d in (self.p("tools", "git", "cmd"), self.p("tools", "python"), self.p("tools", "python", "Scripts")) if os.path.isdir(d)]
        env["PATH"] = os.pathsep.join(extra + [env.get("PATH", "")])
        return env


# ------------------------------------------------------------------------------------------------ small helpers
def safe_extract_zip(zip_path, dest):
    base = os.path.realpath(dest)
    with zipfile.ZipFile(zip_path) as z:
        for m in z.namelist():
            target = os.path.realpath(os.path.join(dest, m))
            if target != base and not target.startswith(base + os.sep):
                raise OpenRowError("ZIP_TRAVERSAL member %s leaves the destination" % m)
        z.extractall(dest)


def tool_note(ctx, name, **kw):
    ctx.receipt["tools"][name] = kw
    ctx.log("TOOL", "%s %s" % (name, " ".join("%s=%s" % (k, v) for k, v in sorted(kw.items()) if k != "detail")))


def user_tool(ctx, name):
    """A tool the user named with --tool NAME=PATH: trusted, its hash recorded, never verified (owner addendum, 2026-11-17)."""
    path = ctx.trust.user_tools.get(name)
    if not path:
        return None
    sha = sha256_file(path) if os.path.isfile(path) else "directory"
    tool_note(ctx, name, anchor="user-supplied", verified="trusted-by-user", sha256=sha, path=path)
    return path


def anchored(ctx, tool, verify):
    """Run `verify()` unless the user explicitly lifted the anchors; return the status string for the receipt."""
    if ctx.trust.accept_unanchored:
        return "lifted: %s" % ctx.trust.accept_unanchored
    verify()
    return "verified"


def signature_check(ctx, path, signer):
    r = ctx.runner.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", os.path.join(HERE, "open_signature.ps1"), "-Path", path], timeout=120)
    if r.rc != 0:
        raise OpenRowError("SIGNATURE_UNREADABLE Get-AuthenticodeSignature failed rc=%s" % r.rc)
    return P.authenticode_check(r.out.strip().splitlines()[-1], signer)


# ------------------------------------------------------------------------------------------------ toolchain
def ensure_git(ctx):
    u = user_tool(ctx, "git")
    if u:
        return u
    spec = ctx.pins["tools"]["git"]
    exe = ctx.p("tools", "git", "cmd", "git.exe")
    if os.path.exists(exe):
        tool_note(ctx, "git", anchor=spec["anchor"], verified="already-installed", release=spec["release_tag"])
        return exe
    rel = ctx.fetcher.get_text(spec["release_api"], accept="application/vnd.github+json")
    url, expected = P.expected_from_github_release(rel, spec["asset"], spec["download_prefix"])
    zp = ctx.p("downloads", spec["asset"])
    got = ctx.fetcher.download(url, zp)
    status = anchored(ctx, "git", lambda: P.verify_sha256(got, expected))
    safe_extract_zip(zp, ctx.p("tools", "git"))
    tool_note(ctx, "git", anchor=spec["anchor"], verified=status, sha256=got, release=spec["release_tag"])
    return exe


def ensure_python(ctx):
    u = user_tool(ctx, "python")
    if u:
        return u
    spec = ctx.pins["tools"]["python"]
    exe = ctx.p("tools", "python", "python.exe")
    if os.path.exists(exe):
        tool_note(ctx, "python", anchor=spec["anchor"], verified="already-installed")
        return exe
    inst = ctx.p("downloads", os.path.basename(spec["url"]))
    got = ctx.fetcher.download(spec["url"], inst)
    status = anchored(ctx, "python", lambda: signature_check(ctx, inst, spec["signer"]))
    dest = ctx.p("tools", "python")
    r = ctx.runner.run([inst] + [a.replace("{dest}", dest) for a in spec["install_args"]], timeout=1800)
    if r.rc != 0 or not os.path.exists(exe):
        raise OpenRowError("INSTALL_FAILED python installer rc=%s, no %s" % (r.rc, exe))
    tool_note(ctx, "python", anchor=spec["anchor"], verified=status, sha256=got)
    return exe


def ensure_lxml(ctx, python):
    spec = ctx.pins["tools"]["lxml"]
    if user_tool(ctx, "lxml"):
        return
    probe = ctx.runner.run([python, "-c", "import lxml, sys; print(lxml.__version__)"], env=ctx.env(), timeout=60)
    if probe.rc == 0 and probe.out.strip() == spec["version"]:
        tool_note(ctx, "lxml", anchor=spec["anchor"], verified="already-installed", version=spec["version"])
        return
    doc = ctx.fetcher.get_text(spec["pypi_json"])
    fname, url, sha = P.pypi_wheel_hash(doc, spec["version"])
    req = ctx.p("downloads", "lxml-requirements.txt")
    os.makedirs(os.path.dirname(req), exist_ok=True)
    with open(req, "w", newline="\n") as f:
        f.write(P.requirements_line("lxml", spec["version"], sha))
    r = ctx.runner.run([python, "-m", "pip", "install", "--require-hashes", "--only-binary=:all:", "--no-deps", "-r", req], env=ctx.env(), timeout=900)
    if r.rc != 0:
        raise OpenRowError("INSTALL_FAILED pip install lxml rc=%s" % r.rc)
    tool_note(ctx, "lxml", anchor=spec["anchor"], verified="pip --require-hashes", version=spec["version"], wheel=fname)


def ensure_nuget(ctx):
    u = user_tool(ctx, "nuget")
    if u:
        return u
    spec = ctx.pins["tools"]["nuget"]
    exe = ctx.p("tools", "nuget", "nuget.exe")
    if os.path.exists(exe):
        tool_note(ctx, "nuget", anchor=spec["anchor"], verified="already-installed")
        return exe
    got = ctx.fetcher.download(spec["url"], exe)
    status = anchored(ctx, "nuget", lambda: signature_check(ctx, exe, spec["signer"]))
    tool_note(ctx, "nuget", anchor=spec["anchor"], verified=status, sha256=got)
    return exe


def find_vs(ctx):
    """(installation path) of a Visual Studio / Build Tools instance with the MSVC x64 tools, or None."""
    vw = os.path.expandvars(VSWHERE)
    r = ctx.runner.run([vw, "-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"], timeout=120)
    path = r.out.strip().splitlines()[0].strip() if r.rc == 0 and r.out.strip() else ""
    return path or None


def vs_tools(vs):
    return {"msbuild": os.path.join(vs, "MSBuild", "Current", "Bin", "MSBuild.exe"),
            "cmake": os.path.join(vs, "Common7", "IDE", "CommonExtensions", "Microsoft", "CMake", "CMake", "bin", "cmake.exe")}


def msvc_install_argv(spec, installer, dest):
    argv = [installer]
    for a in spec["install_args"]:
        argv.append(a.replace("{dest}", dest))
    for c in spec["components"]:
        argv += ["--add", c]
    return argv


def ensure_msvc(ctx):
    spec = ctx.pins["tools"]["msvc"]
    if user_tool(ctx, "msvc"):                              # --tool msvc=installed: the user brought a Visual Studio
        ctx.vs = find_vs(ctx)
        if not ctx.vs:
            raise OpenRowError("TOOL_MISSING --tool msvc given but vswhere finds no MSVC x64 tools")
        return vs_tools(ctx.vs)
    vs = find_vs(ctx)
    if vs and all(os.path.exists(p) for p in vs_tools(vs).values()):
        ctx.vs = vs
        tool_note(ctx, "msvc", anchor=spec["anchor"], verified="already-installed", path=vs)
        return vs_tools(vs)
    inst = ctx.p("downloads", "vs_BuildTools.exe")
    got = ctx.fetcher.download(spec["url"], inst)
    status = anchored(ctx, "msvc", lambda: signature_check(ctx, inst, spec["signer"]))
    dest = ctx.p("tools", "vs")
    r = ctx.runner.run(msvc_install_argv(spec, inst, dest), timeout=7200)
    if r.rc not in spec["ok_exit_codes"]:
        raise OpenRowError("INSTALL_FAILED vs_BuildTools rc=%s (0 and 3010 are success; logs are in %%TEMP%%\\dd_*)" % r.rc)
    ctx.vs = find_vs(ctx)
    if not ctx.vs:
        raise OpenRowError("INSTALL_FAILED vs_BuildTools finished but vswhere finds no MSVC x64 tools")
    tool_note(ctx, "msvc", anchor=spec["anchor"], verified=status, sha256=got, exit=r.rc)
    return vs_tools(ctx.vs)


def ensure_toolchain(ctx, need):
    """need: iterable of tool names. Returns {name: path or dict}. Order matters: python before lxml."""
    out = {}
    if "git" in need:
        out["git"] = ensure_git(ctx)
    if "python" in need or "lxml" in need:
        out["python"] = ensure_python(ctx)
    if "lxml" in need:
        ensure_lxml(ctx, out["python"])
    if "msvc" in need:
        out["msvc"] = ensure_msvc(ctx)
    if "nuget" in need:
        out["nuget"] = ensure_nuget(ctx)
    return out


# ------------------------------------------------------------------------------------------------ sources
def git(ctx, exe, args, cwd=None, timeout=3600):
    return ctx.runner.run([exe] + args, cwd=cwd, env=ctx.env(), timeout=timeout)


def fetch_source(ctx, name, git_exe):
    """A clone of the pinned upstream commit under <root>/src/<name>; refuses a tree that is not at the commit."""
    src = ctx.pins["sources"][name]
    commit = P.resolve_commit(ctx.pins, name, ctx.trust)
    d = ctx.p("src", name)
    if os.path.isdir(os.path.join(d, ".git")):
        head = git(ctx, git_exe, ["rev-parse", "HEAD"], cwd=d, timeout=60).out
        P.check_commit(name, head, commit)
    else:
        os.makedirs(d, exist_ok=True)
        for args in (["init", "-q"], ["remote", "add", "origin", src["repository"]]):
            r = git(ctx, git_exe, args, cwd=d, timeout=120)
            if r.rc != 0:
                raise OpenRowError("GIT_FAILED git %s rc=%s" % (args[0], r.rc))
        flt = ["--filter=" + src["filter"]] if src.get("filter") else []
        r = git(ctx, git_exe, ["fetch", "-q", "--depth", "1"] + flt + ["origin", commit], cwd=d)
        if r.rc != 0 and src.get("tag"):                    # a host that refuses a fetch by sha: the tag, still verified by sha below
            r = git(ctx, git_exe, ["fetch", "-q", "--depth", "1"] + flt + ["origin", "tag", src["tag"]], cwd=d)
        if r.rc != 0:
            raise OpenRowError("GIT_FAILED fetch of %s %s rc=%s" % (name, commit[:12], r.rc))
        dirs = P.sparse_dirs(ctx.pins, name)
        if dirs:
            r = git(ctx, git_exe, ["sparse-checkout", "set", "--cone"] + dirs, cwd=d, timeout=300)
            if r.rc != 0:
                raise OpenRowError("GIT_FAILED sparse-checkout rc=%s" % r.rc)
        r = git(ctx, git_exe, ["checkout", "-q", "--detach", commit], cwd=d, timeout=1800)
        if r.rc != 0:
            raise OpenRowError("GIT_FAILED checkout %s rc=%s" % (commit[:12], r.rc))
        P.check_commit(name, git(ctx, git_exe, ["rev-parse", "HEAD"], cwd=d, timeout=60).out, commit)
    dirty = git(ctx, git_exe, ["status", "--porcelain", "--untracked-files=no"], cwd=d, timeout=300).out.strip()
    if dirty:
        raise OpenRowError("SOURCE_DIRTY tracked files of %s are modified; refusing to build a tree that is not the pinned commit" % name)
    ctx.receipt["sources"][name] = {"repository": src["repository"], "commit": commit, "pinned": commit == src["commit"], "verified": "git rev-parse HEAD"}
    ctx.log("SOURCE", "%s %s pinned=%s" % (name, commit, commit == src["commit"]))
    return d


def fetch_cts_externals(ctx, src_dir, python):
    """CTS's own external/fetch_sources.py: its git revisions and archive sha256 values are in the pinned commit (reviewable upstream)."""
    r = ctx.runner.run([python, os.path.join("external", "fetch_sources.py")], cwd=src_dir, env=ctx.env(), timeout=5400)
    if r.rc != 0:
        raise OpenRowError("FETCH_SOURCES_FAILED external/fetch_sources.py rc=%s" % r.rc)


# ------------------------------------------------------------------------------------------------ builds
def recipe_hash(pins, key):
    body = {"commit": pins["sources"]["cts" if key.startswith("cts") else "dxs"]["commit"], "build": pins["cts_build"] if key.startswith("cts") else pins["dxs_samples"].get(key.split(":", 1)[-1]),
            "nuget_trust": pins["nuget_trust"] if not key.startswith("cts") else None, "key": key}
    return sha256_text(canonical(body))


def locate(build_dir, rel):
    """The expected path of a built exe; the file is searched by name below the build dir if the generator put it elsewhere."""
    p = os.path.join(build_dir, *rel.split("/"))
    if os.path.exists(p):
        return p
    name = os.path.basename(rel)
    for dp, _, files in os.walk(build_dir):
        if name in files:
            return os.path.join(dp, name)
    return None


def cts_configure_argv(cmake, pins, src, build, python):
    b = pins["cts_build"]
    return [cmake, "-S", src, "-B", build, "-G", b["generator"], "-A", b["arch"]] + list(b["cmake_args"]) + ["-DPYTHON_EXECUTABLE=" + python]


def cts_build_argv(cmake, pins, build, target, jobs):
    return [cmake, "--build", build, "--config", pins["cts_build"]["config"], "--target", target, "--parallel", str(jobs)]


def build_cts(ctx, targets, tools, src_dir):
    """Configure once, build each target. Returns {target: (exe, cwd)}. A tree the recipe hash says is built is not rebuilt."""
    build = ctx.p("build", "cts")
    if os.path.realpath(build).startswith(os.path.realpath(src_dir) + os.sep):
        raise OpenRowError("BUILD_DIR_INSIDE_SOURCE the CTS build directory must be outside the sources (upstream README, Windows)")
    out, cmake = {}, tools["msvc"]["cmake"]
    prior = load_json(ctx.p("out", "build-receipt.json")) if os.path.exists(ctx.p("out", "build-receipt.json")) else {"builds": {}}
    todo = []
    for t in targets:
        spec = ctx.pins["cts_build"]["targets"][t]
        exe = locate(build, spec["exe"]) if os.path.isdir(build) else None
        rh = recipe_hash(ctx.pins, "cts:" + t)
        if exe and prior.get("builds", {}).get("cts:" + t, {}).get("recipe_sha256") == rh:
            ctx.receipt["builds"]["cts:" + t] = prior["builds"]["cts:" + t]
            out[t] = (exe, os.path.join(build, *spec["cwd"].split("/")))
            ctx.log("BUILD", "cts:%s already built (recipe %s)" % (t, rh[:12]))
        else:
            todo.append(t)
    if todo:
        r = ctx.runner.run(cts_configure_argv(cmake, ctx.pins, src_dir, build, tools["python"]), env=ctx.env(), timeout=1800)
        if r.rc != 0:
            raise OpenRowError("CMAKE_CONFIGURE_FAILED rc=%s: %s" % (r.rc, r.out[-300:].replace("\n", " ")))
        for t in todo:
            r = ctx.runner.run(cts_build_argv(cmake, ctx.pins, build, t, ctx.jobs), env=ctx.env(), timeout=14400)
            if r.rc != 0:
                raise OpenRowError("BUILD_FAILED cts target %s rc=%s: %s" % (t, r.rc, r.out[-300:].replace("\n", " ")))
            spec = ctx.pins["cts_build"]["targets"][t]
            exe = locate(build, spec["exe"])
            if not exe:
                raise OpenRowError("BUILD_FAILED cts target %s produced no %s" % (t, spec["exe"]))
            ctx.receipt["builds"]["cts:" + t] = {"exe": exe, "sha256": sha256_file(exe), "recipe_sha256": recipe_hash(ctx.pins, "cts:" + t), "secs": round(r.secs)}
            out[t] = (exe, os.path.join(build, *spec["cwd"].split("/")))
    return out


def nuget_install_argv(nuget, packages_config, pkg_dir, config):
    return [nuget, "install", packages_config, "-OutputDirectory", pkg_dir, "-ConfigFile", config, "-NonInteractive"]


def msbuild_argv(msbuild, project):
    return [msbuild, project, "/nologo", "/m:2", "/v:minimal", "/t:Build", "/p:Configuration=Release", "/p:Platform=x64", "/p:PlatformToolset=v143"]


def build_dxs(ctx, key, tools, src_dir):
    """Restore the sample's packages with signature validation required, build it with MSBuild. Returns the exe path."""
    spec = ctx.pins["dxs_samples"][key]
    proj = os.path.join(src_dir, *spec["project"].split("/"))
    pdir = os.path.dirname(proj)
    exe = os.path.normpath(os.path.join(pdir, *spec["exe"].split("/")))
    rh = recipe_hash(ctx.pins, "dxs:" + key)
    prior = load_json(ctx.p("out", "build-receipt.json")) if os.path.exists(ctx.p("out", "build-receipt.json")) else {"builds": {}}
    if os.path.exists(exe) and prior.get("builds", {}).get("dxs:" + key, {}).get("recipe_sha256") == rh:
        ctx.receipt["builds"]["dxs:" + key] = prior["builds"]["dxs:" + key]
        ctx.log("BUILD", "dxs:%s already built (recipe %s)" % (key, rh[:12]))
        return exe
    cfg = ctx.p("out", "nuget.config")
    os.makedirs(os.path.dirname(cfg), exist_ok=True)
    with open(cfg, "w", encoding="utf-8", newline="\n") as f:
        f.write(P.nuget_config_xml(ctx.pins["nuget_trust"]))
    pkgdir = os.path.normpath(os.path.join(pdir, *spec["packages_dir"].split("/")))
    r = ctx.runner.run(nuget_install_argv(tools["nuget"], os.path.join(pdir, "packages.config"), pkgdir, cfg), cwd=pdir, env=ctx.env(), timeout=1800)
    if r.rc != 0:
        raise OpenRowError("NUGET_FAILED restore for %s rc=%s (signature validation is required; see the design doc if nuget.org rotated its certificates): %s"
                           % (key, r.rc, r.out[-300:].replace("\n", " ")))
    r = ctx.runner.run(msbuild_argv(tools["msvc"]["msbuild"], proj), cwd=pdir, env=ctx.env(), timeout=7200)
    if r.rc != 0 or not os.path.exists(exe):
        raise OpenRowError("BUILD_FAILED sample %s rc=%s: %s" % (key, r.rc, r.out[-300:].replace("\n", " ")))
    ctx.receipt["builds"]["dxs:" + key] = {"exe": exe, "sha256": sha256_file(exe), "recipe_sha256": rh, "secs": round(r.secs)}
    return exe


def build_soak(ctx, tools):
    cmake, build = tools["msvc"]["cmake"], ctx.p("build", "soak")
    r = ctx.runner.run([cmake, "-S", os.path.join(HERE, "soak"), "-B", build, "-G", ctx.pins["cts_build"]["generator"], "-A", ctx.pins["cts_build"]["arch"]], env=ctx.env(), timeout=900)
    if r.rc == 0:
        r = ctx.runner.run([cmake, "--build", build, "--config", "Release"], env=ctx.env(), timeout=900)
    exe = locate(build, "Release/kf_gldual.exe")
    if r.rc != 0 or not exe:
        raise OpenRowError("BUILD_FAILED kf_gldual rc=%s: %s" % (r.rc, r.out[-300:].replace("\n", " ")))
    ctx.receipt["builds"]["soak"] = {"exe": exe, "sha256": sha256_file(exe), "recipe_sha256": sha256_file(os.path.join(HERE, "kf_gldual.c"))}
    return exe


def write_receipt(ctx):
    path = ctx.p("out", "build-receipt.json")
    prior = load_json(path) if os.path.exists(path) else {}
    merged = dict(prior)
    for k in ("schema", "date", "trust"):
        merged[k] = ctx.receipt[k]
    for k in ("sources", "tools", "builds"):
        merged[k] = dict(prior.get(k, {}), **ctx.receipt[k])
    write_json(path, merged)
    return path


def plan(pins, row_id):
    """What a row needs and where each download is anchored: printed by `kfopen plan` and used for the design doc's table."""
    row = pins["rows"][row_id]
    need = ROW_NEEDS[row["kind"]]
    steps = []
    for t in need:
        spec = pins["tools"][t]
        steps.append({"tool": t, "anchor": spec["anchor"], "url": spec.get("url") or spec.get("release_api") or spec.get("pypi_json")})
    if row["group"] == "cts" and row["kind"] != "gl-soak":
        steps.append({"source": "cts", "anchor": "git-commit", "commit": pins["sources"]["cts"]["commit"]})
    if row["group"] == "dxs" and row["kind"] != "not-run":
        steps.append({"source": "dxs", "anchor": "git-commit", "commit": pins["sources"]["dxs"]["commit"]})
        steps.append({"tool": "nuget-packages", "anchor": "nuget-signature", "url": pins["nuget_trust"]["source"]})
    return steps
