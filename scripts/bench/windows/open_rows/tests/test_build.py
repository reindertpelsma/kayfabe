# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Toolchain, pinned sources and builds (kfopen_build.py) with a fake runner and a fake fetcher: no network, no Windows, no process is started."""
import hashlib
import io
import json
import os
import tempfile
import unittest
import zipfile

import helpers as H
import kfopen_build as B
import kfopen_pins as P
from kfopen_common import OpenRowError, Result, Trust

PINS = P.load_pins()
GIT_TAG = PINS["tools"]["git"]
SIG_OK = '{"Status":"Valid","Subject":"CN=Microsoft Corporation, O=Microsoft Corporation"}'
SIG_PSF = '{"Status":"Valid","Subject":"CN=Python Software Foundation, O=Python Software Foundation"}'


def zip_bytes(members):
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        for name, data in members.items():
            z.writestr(name, data)
    return buf.getvalue()


def release_for(data, body_hash=None, digest=None):
    rel = H.fixture_json("github_release_mingit.json")
    sha = hashlib.sha256(data).hexdigest()
    rel["body"] = "Filename | SHA-256\n--- | ---\nMinGit-2.56.1-64-bit.zip | %s\n" % (body_hash or sha)
    rel["assets"][0]["digest"] = "sha256:" + (digest or sha)
    return json.dumps(rel)


def make_ctx(tmp, runner=None, fetcher=None, trust=None):
    return B.Ctx(PINS, root=tmp, runner=runner or H.FakeRunner(), fetcher=fetcher or H.FakeFetcher(), trust=trust or Trust(), log=lambda *a: None, jobs=2)


def sig_script(json_text):
    return (lambda argv: any(a.endswith("open_signature.ps1") for a in argv), Result(0, json_text))


class Tmp(unittest.TestCase):
    def setUp(self):
        self._t = tempfile.TemporaryDirectory(prefix="kfopen-test-")
        self.tmp = self._t.name
        self.addCleanup(self._t.cleanup)


class GitTool(Tmp):
    def fetcher(self, data, **kw):
        return H.FakeFetcher({GIT_TAG["release_api"]: release_for(data, **kw)},
                             {GIT_TAG["download_prefix"] + GIT_TAG["asset"]: data})

    def test_mingit_is_downloaded_checked_and_unpacked(self):
        data = zip_bytes({"cmd/git.exe": b"MZ"})
        ctx = make_ctx(self.tmp, fetcher=self.fetcher(data))
        exe = B.ensure_git(ctx)
        self.assertTrue(os.path.exists(exe))
        self.assertEqual("verified", ctx.receipt["tools"]["git"]["verified"])
        self.assertEqual(hashlib.sha256(data).hexdigest(), ctx.receipt["tools"]["git"]["sha256"])

    def test_a_download_that_is_not_what_upstream_lists_is_refused(self):
        data = zip_bytes({"cmd/git.exe": b"MZ"})
        ctx = make_ctx(self.tmp, fetcher=self.fetcher(data, body_hash="9" * 64, digest="9" * 64))
        with self.assertRaisesRegex(OpenRowError, "HASH_MISMATCH"):
            B.ensure_git(ctx)
        self.assertFalse(os.path.exists(os.path.join(self.tmp, "tools", "git", "cmd", "git.exe")))

    def test_ensure_git_refuses_conflicting_upstream_hashes(self):
        data = zip_bytes({"cmd/git.exe": b"MZ"})
        ctx = make_ctx(self.tmp, fetcher=self.fetcher(data, body_hash="9" * 64))
        with self.assertRaisesRegex(OpenRowError, "ANCHOR_CONFLICT"):
            B.ensure_git(ctx)

    def test_lifting_the_anchor_is_explicit_and_recorded(self):
        data = zip_bytes({"cmd/git.exe": b"MZ"})
        ctx = make_ctx(self.tmp, fetcher=self.fetcher(data, body_hash="9" * 64, digest="9" * 64), trust=Trust(accept_unanchored="mirror of the release"))
        B.ensure_git(ctx)
        self.assertEqual("lifted: mirror of the release", ctx.receipt["tools"]["git"]["verified"])
        self.assertEqual("lifted", ctx.receipt["trust"]["mode"])

    def test_unanchored_download_is_refused_by_default(self):
        data = zip_bytes({"cmd/git.exe": b"MZ"})
        ctx = make_ctx(self.tmp, fetcher=self.fetcher(data, body_hash="9" * 64, digest="9" * 64))
        self.assertEqual("git-only", ctx.receipt["trust"]["mode"])
        with self.assertRaises(OpenRowError):
            B.ensure_git(ctx)

    def test_zip_member_outside_destination_refused(self):
        z = os.path.join(self.tmp, "evil.zip")
        open(z, "wb").write(zip_bytes({"../escape.txt": b"x"}))
        with self.assertRaisesRegex(OpenRowError, "ZIP_TRAVERSAL"):
            B.safe_extract_zip(z, os.path.join(self.tmp, "dest"))
        self.assertFalse(os.path.exists(os.path.join(self.tmp, "escape.txt")))

    def test_mingit_zip_with_traversal_refused(self):
        data = zip_bytes({"cmd/git.exe": b"MZ", "../../evil.dll": b"x"})
        ctx = make_ctx(self.tmp, fetcher=self.fetcher(data))
        with self.assertRaisesRegex(OpenRowError, "ZIP_TRAVERSAL"):
            B.ensure_git(ctx)

    def test_user_tool_is_trusted_and_hash_recorded(self):
        mine = os.path.join(self.tmp, "mygit.exe")
        open(mine, "wb").write(b"my own git")
        ctx = make_ctx(self.tmp, trust=Trust(user_tools={"git": mine}))
        self.assertEqual(mine, B.ensure_git(ctx))
        self.assertEqual(hashlib.sha256(b"my own git").hexdigest(), ctx.receipt["tools"]["git"]["sha256"])
        self.assertEqual([], ctx.fetcher.downloads)

    def test_user_tool_receipt_marks_trusted_by_user(self):
        mine = os.path.join(self.tmp, "n.exe")
        open(mine, "wb").write(b"x")
        ctx = make_ctx(self.tmp, trust=Trust(user_tools={"nuget": mine}))
        B.ensure_nuget(ctx)
        self.assertEqual("trusted-by-user", ctx.receipt["tools"]["nuget"]["verified"])
        self.assertEqual("user-supplied", ctx.receipt["tools"]["nuget"]["anchor"])
        self.assertEqual(hashlib.sha256(b"x").hexdigest(), ctx.receipt["tools"]["nuget"]["sha256"], "the hash of a user-supplied tool is recorded")


class SignedTools(Tmp):
    def test_nuget_is_downloaded_and_its_signature_checked(self):
        spec = PINS["tools"]["nuget"]
        f = H.FakeFetcher(files={spec["url"]: b"nuget"})
        r = H.FakeRunner([sig_script(SIG_OK)])
        ctx = make_ctx(self.tmp, runner=r, fetcher=f)
        exe = B.ensure_nuget(ctx)
        self.assertTrue(exe.endswith("nuget.exe") and os.path.exists(exe))
        self.assertEqual("verified", ctx.receipt["tools"]["nuget"]["verified"])
        self.assertTrue(r.find("open_signature.ps1", "-Path"))

    def test_ensure_nuget_refuses_unsigned_file(self):
        spec = PINS["tools"]["nuget"]
        ctx = make_ctx(self.tmp, runner=H.FakeRunner([sig_script('{"Status":"NotSigned","Subject":""}')]), fetcher=H.FakeFetcher(files={spec["url"]: b"nuget"}))
        with self.assertRaisesRegex(OpenRowError, "SIGNATURE_INVALID"):
            B.ensure_nuget(ctx)

    def test_signature_failure_refuses_by_default(self):
        spec = PINS["tools"]["nuget"]
        ctx = make_ctx(self.tmp, runner=H.FakeRunner([(lambda a: True, Result(1, "boom"))]), fetcher=H.FakeFetcher(files={spec["url"]: b"nuget"}))
        with self.assertRaisesRegex(OpenRowError, "SIGNATURE_UNREADABLE"):
            B.ensure_nuget(ctx)

    def test_ensure_python_refuses_other_signer(self):
        spec = PINS["tools"]["python"]
        r = H.FakeRunner([sig_script(SIG_OK)])         # signed by Microsoft, but python.org must be signed by the PSF
        ctx = make_ctx(self.tmp, runner=r, fetcher=H.FakeFetcher(files={spec["url"]: b"py"}))
        with self.assertRaisesRegex(OpenRowError, "SIGNATURE_SIGNER"):
            B.ensure_python(ctx)
        self.assertFalse([c for c in r.argvs() if c[0].endswith(".exe") and "python-3" in c[0]], "an unverified installer was run")

    def test_python_installer_runs_after_a_good_signature(self):
        spec = PINS["tools"]["python"]
        dest = os.path.join(self.tmp, "tools", "python")

        def install(argv, cwd):
            os.makedirs(dest, exist_ok=True)
            open(os.path.join(dest, "python.exe"), "wb").write(b"py")
            return Result(0, "")
        r = H.FakeRunner([sig_script(SIG_PSF), (lambda a: a[0].endswith("python-3.12.10-amd64.exe"), install)])
        ctx = make_ctx(self.tmp, runner=r, fetcher=H.FakeFetcher(files={spec["url"]: b"py"}))
        exe = B.ensure_python(ctx)
        self.assertTrue(exe.endswith("python.exe"))
        argv = [c for c in r.calls if c["argv"][0].endswith("python-3.12.10-amd64.exe")][0]["argv"]
        self.assertIn("/quiet", argv)
        self.assertIn("TargetDir=" + dest, argv)

    def test_python_installer_failure_is_refused(self):
        spec = PINS["tools"]["python"]
        r = H.FakeRunner([sig_script(SIG_PSF), (lambda a: a[0].endswith("python-3.12.10-amd64.exe"), Result(1603, ""))])
        ctx = make_ctx(self.tmp, runner=r, fetcher=H.FakeFetcher(files={spec["url"]: b"py"}))
        with self.assertRaisesRegex(OpenRowError, "INSTALL_FAILED"):
            B.ensure_python(ctx)


class Lxml(Tmp):
    def run_with(self, pypi_text, probe_rc=1):
        spec = PINS["tools"]["lxml"]
        r = H.FakeRunner([(lambda a: "-c" in a, Result(probe_rc, "")), (lambda a: "pip" in a, Result(0, ""))])
        ctx = make_ctx(self.tmp, runner=r, fetcher=H.FakeFetcher({spec["pypi_json"]: pypi_text}))
        return ctx, r

    def test_lxml_is_installed_with_the_hash_pip_must_enforce(self):
        ctx, r = self.run_with(H.fixture("pypi_lxml.json"))
        B.ensure_lxml(ctx, "python.exe")
        pip = r.find("pip", "install")[0]["argv"]
        self.assertIn("--require-hashes", pip)
        self.assertIn("--only-binary=:all:", pip)
        req = open(os.path.join(self.tmp, "downloads", "lxml-requirements.txt")).read()
        self.assertEqual("lxml==6.1.3 --hash=sha256:" + "3" * 64 + "\n", req)

    def test_lxml_uses_hash_of_pinned_version_only(self):
        doc = H.fixture_json("pypi_lxml.json")
        doc["info"]["version"] = "7.0.0b1"
        ctx, r = self.run_with(json.dumps(doc))
        with self.assertRaisesRegex(OpenRowError, "ANCHOR_MISSING"):
            B.ensure_lxml(ctx, "python.exe")
        self.assertFalse(r.find("pip", "install"))

    def test_lxml_wheel_on_another_host_refused(self):
        doc = H.fixture_json("pypi_lxml.json")
        doc["urls"][0]["url"] = "https://evil.example/lxml.whl"
        ctx, _ = self.run_with(json.dumps(doc))
        with self.assertRaisesRegex(OpenRowError, "ANCHOR_ORIGIN"):
            B.ensure_lxml(ctx, "python.exe")

    def test_lxml_two_wheels_refused(self):
        doc = H.fixture_json("pypi_lxml.json")
        doc["urls"].append(json.loads(json.dumps(doc["urls"][0])))
        ctx, r = self.run_with(json.dumps(doc))
        with self.assertRaisesRegex(OpenRowError, "need exactly one"):
            B.ensure_lxml(ctx, "python.exe")
        self.assertFalse(r.find("pip", "install"))

    def test_lxml_already_there_is_not_reinstalled(self):
        spec = PINS["tools"]["lxml"]
        r = H.FakeRunner([(lambda a: "-c" in a, Result(0, spec["version"] + "\n"))])
        ctx = make_ctx(self.tmp, runner=r)
        B.ensure_lxml(ctx, "python.exe")
        self.assertFalse(r.find("pip", "install"))
        self.assertEqual("already-installed", ctx.receipt["tools"]["lxml"]["verified"])


class Msvc(Tmp):
    def vs_runner(self, installer_rc, installed_after=True):
        state = {"installed": False}
        vs = os.path.join(self.tmp, "VS")

        def vswhere(argv, cwd):
            return Result(0, (vs + "\n") if state["installed"] else "")

        def installer(argv, cwd):
            state["installed"] = installed_after and installer_rc in (0, 3010)
            return Result(installer_rc, "")
        r = H.FakeRunner([sig_script(SIG_OK), (lambda a: a[0].endswith("vswhere.exe"), vswhere), (lambda a: a[0].endswith("vs_BuildTools.exe"), installer)])
        return r, vs

    def ctx_for(self, runner):
        spec = PINS["tools"]["msvc"]
        return make_ctx(self.tmp, runner=runner, fetcher=H.FakeFetcher(files={spec["url"]: b"vs"}))

    def test_msvc_exit_3010_is_success(self):
        r, vs = self.vs_runner(3010)
        ctx = self.ctx_for(r)
        tools = B.ensure_msvc(ctx)
        self.assertTrue(tools["cmake"].startswith(vs) and tools["msbuild"].startswith(vs))
        self.assertEqual(3010, ctx.receipt["tools"]["msvc"]["exit"])

    def test_toolchain_prepares_dx_row_with_msvc_reboot_code(self):
        r, _ = self.vs_runner(3010)
        spec_n = PINS["tools"]["nuget"]
        ctx = make_ctx(self.tmp, runner=r, fetcher=H.FakeFetcher(files={PINS["tools"]["msvc"]["url"]: b"vs", spec_n["url"]: b"nuget"}))
        got = B.ensure_toolchain(ctx, ("msvc", "nuget"))
        self.assertIn("msbuild", got["msvc"])
        self.assertTrue(got["nuget"].endswith("nuget.exe"))

    def test_msvc_failure_exit_is_refused(self):
        r, _ = self.vs_runner(5007)
        with self.assertRaisesRegex(OpenRowError, "INSTALL_FAILED"):
            B.ensure_msvc(self.ctx_for(r))

    def test_msvc_install_argv_is_quiet_waits_and_adds_every_component(self):
        r, _ = self.vs_runner(0)
        B.ensure_msvc(self.ctx_for(r))
        argv = r.find("vs_BuildTools.exe", "--quiet")[0]["argv"]
        for flag in ("--quiet", "--wait", "--norestart", "--nocache", "--noUpdateInstaller", "--installPath"):
            self.assertIn(flag, argv)
        adds = [argv[i + 1] for i, a in enumerate(argv) if a == "--add"]
        self.assertEqual(PINS["tools"]["msvc"]["components"], adds)
        self.assertIn("Microsoft.VisualStudio.Component.VC.CMake.Project", adds)

    def test_unsigned_bootstrapper_is_never_run(self):
        r, _ = self.vs_runner(0)
        r.script[0] = sig_script('{"Status":"HashMismatch","Subject":"CN=Microsoft Corporation"}')
        with self.assertRaisesRegex(OpenRowError, "SIGNATURE_INVALID"):
            B.ensure_msvc(self.ctx_for(r))
        self.assertFalse(r.find("--quiet"))

    def test_an_installed_visual_studio_is_reused(self):
        vs = os.path.join(self.tmp, "VS")
        for p in B.vs_tools(vs).values():
            os.makedirs(os.path.dirname(p), exist_ok=True)
            open(p, "wb").write(b"x")
        r = H.FakeRunner([(lambda a: a[0].endswith("vswhere.exe"), Result(0, vs + "\n"))])
        ctx = make_ctx(self.tmp, runner=r)
        B.ensure_msvc(ctx)
        self.assertEqual("already-installed", ctx.receipt["tools"]["msvc"]["verified"])
        self.assertEqual([], ctx.fetcher.downloads)

    def test_vs_tools_paths(self):
        t = B.vs_tools("C:\\VS")
        self.assertTrue(t["msbuild"].endswith(os.path.join("MSBuild", "Current", "Bin", "MSBuild.exe")))
        self.assertIn("CMake", t["cmake"])


class Sources(Tmp):
    COMMIT = PINS["sources"]["cts"]["commit"]

    def git_runner(self, head=None, dirty="", fetch_rc=0):
        head = head or self.COMMIT

        def git(argv, cwd):
            sub = argv[1]
            if sub == "rev-parse":
                return Result(0, head + "\n")
            if sub == "status":
                return Result(0, dirty)
            if sub == "fetch":
                return Result(fetch_rc if "tag" not in argv else 0, "")
            return Result(0, "")
        return H.FakeRunner([(lambda a: a[0].endswith("git.exe") or a[0] == "git", git)])

    def test_fetch_checks_out_the_pinned_commit(self):
        r = self.git_runner()
        ctx = make_ctx(self.tmp, runner=r)
        d = B.fetch_source(ctx, "cts", "git")
        self.assertTrue(d.endswith(os.path.join("src", "cts")))
        fetch = r.find("fetch", "--depth")[0]["argv"]
        self.assertEqual(self.COMMIT, fetch[-1])
        self.assertIn("https://github.com/KhronosGroup/VK-GL-CTS.git", " ".join(a for c in r.argvs() for a in c))
        self.assertTrue(ctx.receipt["sources"]["cts"]["pinned"])

    def test_fetch_source_refuses_wrong_head(self):
        ctx = make_ctx(self.tmp, runner=self.git_runner(head="f" * 40))
        with self.assertRaises(P.PinMismatch):
            B.fetch_source(ctx, "cts", "git")

    def test_fetch_source_refuses_a_tree_at_another_commit(self):
        os.makedirs(os.path.join(self.tmp, "src", "cts", ".git"))
        ctx = make_ctx(self.tmp, runner=self.git_runner(head="f" * 40))
        with self.assertRaisesRegex(P.PinMismatch, "PIN_MISMATCH cts"):
            B.fetch_source(ctx, "cts", "git")

    def test_dirty_tree_refused(self):
        ctx = make_ctx(self.tmp, runner=self.git_runner(dirty=" M external/fetch_sources.py\n"))
        with self.assertRaisesRegex(OpenRowError, "SOURCE_DIRTY"):
            B.fetch_source(ctx, "cts", "git")

    def test_dirty_tree_is_not_built(self):
        os.makedirs(os.path.join(self.tmp, "src", "dxs", ".git"))
        r = self.git_runner(head=PINS["sources"]["dxs"]["commit"], dirty=" M x.cpp\n")
        ctx = make_ctx(self.tmp, runner=r)
        with self.assertRaisesRegex(OpenRowError, "SOURCE_DIRTY"):
            B.fetch_source(ctx, "dxs", "git")
        self.assertFalse(r.find("msbuild"))

    def test_override_builds_other_commit_and_receipt_marks_unpinned(self):
        other = "1" * 40
        ctx = make_ctx(self.tmp, runner=self.git_runner(head=other), trust=Trust(commit_overrides={"cts": other}))
        B.fetch_source(ctx, "cts", "git")
        self.assertFalse(ctx.receipt["sources"]["cts"]["pinned"])
        self.assertEqual(other, ctx.receipt["sources"]["cts"]["commit"])
        self.assertEqual("lifted", ctx.receipt["trust"]["mode"])

    def test_a_host_that_refuses_fetch_by_sha_falls_back_to_the_tag_and_still_verifies(self):
        r = self.git_runner(fetch_rc=1)
        ctx = make_ctx(self.tmp, runner=r)
        B.fetch_source(ctx, "cts", "git")
        self.assertTrue([c for c in r.argvs() if "tag" in c and "vulkan-cts-1.4.7.1" in c])
        r2 = self.git_runner(fetch_rc=1, head="f" * 40)
        with self.assertRaises(P.PinMismatch):
            B.fetch_source(make_ctx(os.path.join(self.tmp, "b"), runner=r2), "cts", "git")

    def test_dxs_is_fetched_as_a_blobless_sparse_checkout_of_the_pinned_commit(self):
        r = self.git_runner(head=PINS["sources"]["dxs"]["commit"])
        ctx = make_ctx(self.tmp, runner=r)
        B.fetch_source(ctx, "dxs", "git")
        fetch = r.find("fetch", "--depth")[0]["argv"]
        self.assertIn("--filter=blob:none", fetch)
        self.assertEqual(PINS["sources"]["dxs"]["commit"], fetch[-1])
        sparse = r.find("sparse-checkout", "--cone")[0]["argv"]
        for spec in PINS["dxs_samples"].values():
            self.assertIn(spec["checkout"], sparse)
        order = [c[1] for c in r.argvs() if len(c) > 1]
        self.assertLess(order.index("sparse-checkout"), order.index("checkout"), "the sparse set is chosen before the files are checked out")

    def test_cts_is_a_full_depth_one_fetch(self):
        r = self.git_runner()
        B.fetch_source(make_ctx(self.tmp, runner=r), "cts", "git")
        self.assertNotIn("--filter=blob:none", r.find("fetch", "--depth")[0]["argv"])
        self.assertFalse(r.find("sparse-checkout"))

    def test_cts_externals_come_from_the_upstream_script_of_the_pinned_commit(self):
        r = H.FakeRunner()
        ctx = make_ctx(self.tmp, runner=r)
        B.fetch_cts_externals(ctx, os.path.join(self.tmp, "src", "cts"), "python.exe")
        self.assertEqual(["python.exe", os.path.join("external", "fetch_sources.py")], r.argvs()[0])
        self.assertEqual(os.path.join(self.tmp, "src", "cts"), r.calls[0]["cwd"])


class Builds(Tmp):
    def tools(self):
        return {"msvc": {"cmake": "cmake.exe", "msbuild": "MSBuild.exe"}, "python": "python.exe", "nuget": "nuget.exe", "git": "git"}

    def cts_runner(self, produce=True, fail_target=None):
        build = os.path.join(self.tmp, "build", "cts")

        def run(argv, cwd):
            if "--target" in argv:
                t = argv[argv.index("--target") + 1]
                if t == fail_target:
                    return Result(1, "error C2065")
                if produce:
                    rel = PINS["cts_build"]["targets"][t]["exe"]
                    p = os.path.join(build, *rel.split("/"))
                    os.makedirs(os.path.dirname(p), exist_ok=True)
                    open(p, "wb").write(b"exe-" + t.encode())
            return Result(0, "")
        return H.FakeRunner([(lambda a: a[0] == "cmake.exe", run)])

    def test_cts_configure_argv_has_pinned_args(self):
        argv = B.cts_configure_argv("cmake.exe", PINS, "S", "B", "python.exe")
        self.assertIn("-DDEQP_TARGET=default", argv)
        self.assertIn("-DDEQP_DISABLE_VK_VIDEO_TESTS=ON", argv)
        self.assertEqual("Visual Studio 17 2022", argv[argv.index("-G") + 1])

    def test_cts_configure_uses_vs2022_x64_and_python(self):
        argv = B.cts_configure_argv("cmake.exe", PINS, "S", "B", "C:\\py\\python.exe")
        self.assertEqual("x64", argv[argv.index("-A") + 1])
        self.assertIn("-DPYTHON_EXECUTABLE=C:\\py\\python.exe", argv)
        self.assertEqual(["-S", "S", "-B", "B"], argv[1:5])

    def test_configure_runs_with_the_pinned_cmake_args_once(self):
        r = self.cts_runner()
        B.build_cts(make_ctx(self.tmp, runner=r), ["glcts"], self.tools(), os.path.join(self.tmp, "src", "cts"))
        configure = r.find("-S")[0]["argv"]
        for a in PINS["cts_build"]["cmake_args"]:
            self.assertIn(a, configure)

    def test_cts_build_argv_builds_release_of_one_target(self):
        argv = B.cts_build_argv("cmake.exe", PINS, "B", "glcts", 4)
        self.assertEqual(["cmake.exe", "--build", "B", "--config", "Release", "--target", "glcts", "--parallel", "4"], argv)

    def test_cts_build_produces_binaries_and_a_receipt_entry(self):
        r = self.cts_runner()
        ctx = make_ctx(self.tmp, runner=r)
        out = B.build_cts(ctx, ["glcts", "deqp-vk"], self.tools(), os.path.join(self.tmp, "src", "cts"))
        self.assertEqual({"glcts", "deqp-vk"}, set(out))
        self.assertTrue(out["glcts"][0].endswith("glcts.exe"))
        self.assertEqual(1, len(r.find("-S")), "one configure for both targets")
        self.assertEqual(2, len(r.find("--target")))
        self.assertEqual(hashlib.sha256(b"exe-glcts").hexdigest(), ctx.receipt["builds"]["cts:glcts"]["sha256"])

    def test_build_dir_inside_source_refused(self):
        ctx = make_ctx(os.path.join(self.tmp, "src", "cts", "root"), runner=self.cts_runner())
        with self.assertRaisesRegex(OpenRowError, "BUILD_DIR_INSIDE_SOURCE"):
            B.build_cts(ctx, ["glcts"], self.tools(), os.path.join(self.tmp, "src", "cts"))

    def test_failed_target_build_refuses(self):
        ctx = make_ctx(self.tmp, runner=self.cts_runner(fail_target="glcts"))
        with self.assertRaisesRegex(OpenRowError, "BUILD_FAILED cts target glcts"):
            B.build_cts(ctx, ["glcts"], self.tools(), os.path.join(self.tmp, "src", "cts"))

    def test_a_build_that_leaves_no_binary_refuses(self):
        ctx = make_ctx(self.tmp, runner=self.cts_runner(produce=False))
        with self.assertRaisesRegex(OpenRowError, "produced no"):
            B.build_cts(ctx, ["glcts"], self.tools(), os.path.join(self.tmp, "src", "cts"))

    def test_a_built_tree_with_the_same_recipe_is_not_rebuilt(self):
        ctx = make_ctx(self.tmp, runner=self.cts_runner())
        B.build_cts(ctx, ["glcts"], self.tools(), os.path.join(self.tmp, "src", "cts"))
        B.write_receipt(ctx)
        r2 = self.cts_runner()
        ctx2 = make_ctx(self.tmp, runner=r2)
        B.build_cts(ctx2, ["glcts"], self.tools(), os.path.join(self.tmp, "src", "cts"))
        self.assertEqual([], r2.calls)

    def test_recipe_hash_changes_with_commit(self):
        other = json.loads(json.dumps(PINS))
        other["sources"]["cts"]["commit"] = "2" * 40
        self.assertNotEqual(B.recipe_hash(PINS, "cts:glcts"), B.recipe_hash(other, "cts:glcts"))
        other = json.loads(json.dumps(PINS))
        other["sources"]["dxs"]["commit"] = "3" * 40
        self.assertNotEqual(B.recipe_hash(PINS, "dxs:nbody"), B.recipe_hash(other, "dxs:nbody"))

    def test_override_commit_forces_rebuild(self):
        ctx = make_ctx(self.tmp, runner=self.cts_runner())
        B.build_cts(ctx, ["glcts"], self.tools(), os.path.join(self.tmp, "src", "cts"))
        B.write_receipt(ctx)
        other = json.loads(json.dumps(PINS))
        other["sources"]["cts"]["commit"] = "4" * 40
        r2 = self.cts_runner()
        ctx2 = B.Ctx(other, root=self.tmp, runner=r2, fetcher=H.FakeFetcher(), trust=Trust(), log=lambda *a: None)
        B.build_cts(ctx2, ["glcts"], self.tools(), os.path.join(self.tmp, "src", "cts"))
        self.assertTrue(r2.find("--target"))

    # ---- DirectX samples
    def dxs_runner(self, nuget_rc=0, msbuild_rc=0, produce=True):
        src = os.path.join(self.tmp, "src", "dxs")
        exe = os.path.join(src, "Samples", "Desktop", "D3D12nBodyGravity", "src", "bin", "x64", "Release", "D3D12nBodyGravity.exe")

        def nuget(argv, cwd):
            return Result(nuget_rc, "NU3037" if nuget_rc else "")

        def msb(argv, cwd):
            if msbuild_rc == 0 and produce:
                os.makedirs(os.path.dirname(exe), exist_ok=True)
                open(exe, "wb").write(b"sample")
            return Result(msbuild_rc, "")
        return H.FakeRunner([(lambda a: a[0] == "nuget.exe", nuget), (lambda a: a[0] == "MSBuild.exe", msb)]), src, exe

    def test_nuget_install_uses_the_signature_config(self):
        r, src, exe = self.dxs_runner()
        ctx = make_ctx(self.tmp, runner=r)
        B.build_dxs(ctx, "nbody", self.tools(), src)
        argv = r.find("nuget.exe", "install")[0]["argv"]
        cfg = argv[argv.index("-ConfigFile") + 1]
        self.assertTrue(os.path.exists(cfg))
        self.assertIn('signatureValidationMode" value="require"', open(cfg).read())
        self.assertIn("-NonInteractive", argv)
        self.assertTrue(argv[2].endswith("packages.config"))

    def test_config_file_written_with_fingerprints(self):
        r, src, exe = self.dxs_runner()
        ctx = make_ctx(self.tmp, runner=r)
        B.build_dxs(ctx, "nbody", self.tools(), src)
        text = open(os.path.join(self.tmp, "out", "nuget.config")).read()
        for f in PINS["nuget_trust"]["fingerprints_sha256"]:
            self.assertIn(f, text)

    def test_nuget_failure_refuses_build(self):
        r, src, exe = self.dxs_runner(nuget_rc=1)
        with self.assertRaisesRegex(OpenRowError, "NUGET_FAILED"):
            B.build_dxs(make_ctx(self.tmp, runner=r), "nbody", self.tools(), src)

    def test_nuget_failure_leaves_no_receipt(self):
        r, src, exe = self.dxs_runner(nuget_rc=1)
        ctx = make_ctx(self.tmp, runner=r)
        with self.assertRaises(OpenRowError):
            B.build_dxs(ctx, "nbody", self.tools(), src)
        self.assertEqual({}, ctx.receipt["builds"])
        self.assertFalse(r.find("MSBuild.exe"), "msbuild ran although the restore failed")

    def test_msbuild_argv_is_release_x64_v143(self):
        argv = B.msbuild_argv("MSBuild.exe", "p.vcxproj")
        for a in ("/p:Configuration=Release", "/p:Platform=x64", "/p:PlatformToolset=v143", "/t:Build"):
            self.assertIn(a, argv)

    def test_sample_build_returns_the_exe_and_records_its_hash(self):
        r, src, exe = self.dxs_runner()
        ctx = make_ctx(self.tmp, runner=r)
        got = B.build_dxs(ctx, "nbody", self.tools(), src)
        self.assertEqual(os.path.normpath(exe), os.path.normpath(got))
        self.assertEqual(hashlib.sha256(b"sample").hexdigest(), ctx.receipt["builds"]["dxs:nbody"]["sha256"])

    def test_msbuild_failure_and_missing_exe_refuse(self):
        r, src, exe = self.dxs_runner(msbuild_rc=1)
        with self.assertRaisesRegex(OpenRowError, "BUILD_FAILED sample nbody"):
            B.build_dxs(make_ctx(self.tmp, runner=r), "nbody", self.tools(), src)
        r, src, exe = self.dxs_runner(produce=False)
        with self.assertRaisesRegex(OpenRowError, "BUILD_FAILED sample nbody"):
            B.build_dxs(make_ctx(os.path.join(self.tmp, "b"), runner=r), "nbody", self.tools(), src)

    def test_work_graphs_packages_live_one_level_up(self):
        r = H.FakeRunner([(lambda a: a[0] == "nuget.exe", Result(0, "")), (lambda a: a[0] == "MSBuild.exe", Result(1, ""))])
        ctx = make_ctx(self.tmp, runner=r)
        with self.assertRaises(OpenRowError):
            B.build_dxs(ctx, "work_graphs", self.tools(), os.path.join(self.tmp, "src", "dxs"))
        out = r.find("nuget.exe")[0]["argv"]
        pkg = out[out.index("-OutputDirectory") + 1]
        self.assertTrue(pkg.endswith(os.path.join("D3D12HelloWorld", "src", "packages")), pkg)

    def test_soak_is_built_with_cmake_and_hashed(self):
        build = os.path.join(self.tmp, "build", "soak")

        def run(argv, cwd):
            if "--build" in argv:
                p = os.path.join(build, "Release", "kf_gldual.exe")
                os.makedirs(os.path.dirname(p), exist_ok=True)
                open(p, "wb").write(b"soak")
            return Result(0, "")
        ctx = make_ctx(self.tmp, runner=H.FakeRunner([(lambda a: a[0] == "cmake.exe", run)]))
        exe = B.build_soak(ctx, self.tools())
        self.assertTrue(exe.endswith("kf_gldual.exe"))
        self.assertIn("soak", ctx.receipt["builds"])

    def test_receipt_merges_with_earlier_builds(self):
        r, src, exe = self.dxs_runner()
        ctx = make_ctx(self.tmp, runner=r)
        B.build_dxs(ctx, "nbody", self.tools(), src)
        path = B.write_receipt(ctx)
        ctx2 = make_ctx(self.tmp)
        ctx2.receipt["builds"]["soak"] = {"exe": "x", "sha256": "s", "recipe_sha256": "r"}
        B.write_receipt(ctx2)
        doc = json.load(open(path))
        self.assertTrue({"dxs:nbody", "soak"} <= set(doc["builds"]))


class Plan(unittest.TestCase):
    def test_every_download_of_a_row_names_its_anchor(self):
        steps = B.plan(PINS, "vkglcts_vk")
        anchors = {s.get("tool") or s.get("source"): s["anchor"] for s in steps}
        self.assertEqual({"git": "upstream-sha256", "python": "authenticode", "lxml": "pypi-sha256", "msvc": "authenticode", "cts": "git-commit"}, anchors)

    def test_dx_rows_restore_nuget_packages_with_signatures(self):
        steps = B.plan(PINS, "dxsamples_nbody_async")
        self.assertIn("nuget-signature", [s["anchor"] for s in steps])
        self.assertIn("dxs", [s.get("source") for s in steps])

    def test_not_run_row_downloads_nothing(self):
        self.assertEqual([], B.plan(PINS, "dxsamples_multiadapter"))

    def test_soak_only_needs_the_msvc_toolchain(self):
        self.assertEqual(["msvc"], [s.get("tool") for s in B.plan(PINS, "vkglcts_gl_dual_soak")])


if __name__ == "__main__":
    unittest.main()
