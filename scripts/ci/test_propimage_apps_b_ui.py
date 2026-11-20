#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of the image B launch rows' discovery and UI text capture (2026-11-20, TEST BRANCH for a Vast run).

STATUS: LIVE, 2026-11-20.

What is tested without a Windows guest: the launch table (discovery rows carry file-name patterns, never a versioned install path), the
host side of the capture (which files are pulled, their bound, the scrub of a pulled file, the `ui_hits` count in the grade) and a
brace/quote balance guard for the guest scripts (there is no PowerShell on a Linux runner, `PARSE_CHECK` runs in the guest only). What
is NOT tested here: that Windows PowerShell 5.1 parses and runs the scripts and that UI Automation returns text (first run on a box)."""
import json
import re
import sys
import tempfile
import types
import unittest
import unittest.mock
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "bench/windows"))
sys.path.insert(0, str(HERE))
import propimage_apps_b as B  # noqa: E402

LAUNCH = B.APPS_DIR / "launch.ps1"
NEW_DISCOVERY = ("dreamweaver", "incopy", "substance_designer", "substance_modeler", "substance_painter", "substance_sampler", "substance_stager",
                 "mediaencoderlaunch", "animatelaunch")


def ps_balance(text):
    """Cheap guard for a PowerShell script without PowerShell: returns '' when braces, parentheses and brackets balance outside of strings
    and comments and no string, here-string or block comment is left open; else a message with the line number."""
    stack, i, n, line = [], 0, len(text), 1
    pairs = {")": "(", "]": "[", "}": "{"}
    while i < n:
        ch = text[i]
        if ch == "\n":
            line += 1
        if text.startswith("<#", i):
            j = text.find("#>", i + 2)
            if j < 0:
                return "line %d: block comment not closed" % line
            line += text.count("\n", i, j)
            i = j + 2
            continue
        if ch == "@" and i + 1 < n and text[i + 1] in "'\"" and re.match(r"[ \t]*\r?\n", text[i + 2:i + 6] or "x"):
            q = text[i + 1]
            m = re.compile(r"\n" + q + "@").search(text, i + 2)
            if not m:
                return "line %d: here-string not closed" % line
            line += text.count("\n", i, m.end())
            i = m.end()
            continue
        if ch == "#":
            while i < n and text[i] != "\n":
                i += 1
            continue
        if ch == "'":
            i += 1
            while i < n:
                if text[i] == "'" and text[i + 1:i + 2] == "'":
                    i += 2
                    continue
                if text[i] == "'":
                    break
                if text[i] == "\n":
                    line += 1
                i += 1
            else:
                return "line %d: single-quoted string not closed" % line
            i += 1
            continue
        if ch == '"':
            i += 1
            while i < n:
                if text[i] == "`":
                    i += 2
                    continue
                if text[i] == '"' and text[i + 1:i + 2] == '"':
                    i += 2
                    continue
                if text[i] == '"':
                    break
                if text[i] == "\n":
                    line += 1
                i += 1
            else:
                return "line %d: double-quoted string not closed" % line
            i += 1
            continue
        if ch == "`":
            i += 2
            continue
        if ch in "([{":
            stack.append((ch, line))
        elif ch in ")]}":
            if not stack or stack[-1][0] != pairs[ch]:
                return "line %d: unbalanced %s" % (line, ch)
            stack.pop()
        i += 1
    return "line %d: %s not closed" % (stack[-1][1], stack[-1][0]) if stack else ""


class Balance(unittest.TestCase):
    def test_the_guard_itself(self):
        self.assertEqual(ps_balance("function f { if ($a) { 'x}' } }\n# }\n"), "")
        self.assertEqual(ps_balance("$h = @{ a = (1 + 2); b = @('x', \"y(\") }\n"), "")
        self.assertEqual(ps_balance("$s = @'\n{ not code\n'@\n$t = 1\n"), "")
        self.assertIn("not closed", ps_balance("function f { if ($a) { 1 }\n"))
        self.assertIn("unbalanced", ps_balance("function f { 1 } }\n"))
        self.assertIn("unbalanced", ps_balance("$a = (1 + 2]\n"))
        self.assertIn("single-quoted", ps_balance("$a = 'abc\n"))
        self.assertIn("here-string", ps_balance("$s = @'\nabc\n"))
        self.assertEqual(ps_balance("$a = 'it''s'\n"), "")

    def test_every_guest_script_balances(self):
        for f in sorted(B.APPS_DIR.glob("*.ps1")):
            self.assertEqual(ps_balance(f.read_text()), "", f.name)

    def test_every_assembled_row_script_balances(self):
        for name in B.CHECKS:
            self.assertEqual(ps_balance(B.render_check(name, "a" * 32)), "", name)


class LaunchTable(unittest.TestCase):
    def setUp(self):
        self.ps1 = LAUNCH.read_text()

    def entry(self, name):
        m = re.search(r"(?m)^\s*%s\s*=\s*@\{(.*)\}\s*$" % name, self.ps1)
        self.assertTrue(m, name)
        return m.group(1)

    def test_discovery_rows_are_launch_only_discovery_rows_with_a_short_timeout(self):
        for name in NEW_DISCOVERY:
            c = B.CHECKS[name]
            self.assertIn(name, B.LAUNCH_ONLY)
            self.assertEqual((c["lane"], c["script"], c["gpu"]), ("discovery", "launch.ps1", ()), name)
            self.assertLessEqual(c["timeout"], 900, name)
            self.assertFalse(c.get("disabled"), name)

    def test_discovery_rows_discover_the_exe_and_carry_no_versioned_path(self):
        for name in NEW_DISCOVERY:
            e = self.entry(name)
            self.assertIn("exe = $null", e, name)
            self.assertRegex(e, r"find = @\('[^']+\.exe'", name)
            self.assertNotIn("Program Files", e, name)
            self.assertNotRegex(e, r"20\d\d", name)

    def test_the_patterns_are_the_requested_ones(self):
        want = {"dreamweaver": "Dreamweaver*.exe", "incopy": "InCopy*.exe", "substance_painter": "*Substance*3D*Painter*.exe",
                "substance_designer": "*Substance*3D*Designer*.exe", "substance_modeler": "*Substance*3D*Modeler*.exe",
                "substance_sampler": "*Substance*3D*Sampler*.exe", "substance_stager": "*Substance*3D*Stager*.exe"}
        for name, pattern in want.items():
            self.assertIn("'%s'" % pattern, self.entry(name), name)

    def test_the_search_roots_depth_and_the_named_failure(self):
        self.assertIn("Get-ChildItem -LiteralPath 'C:\\Program Files' -Directory -Filter 'Adobe*'", self.ps1)
        self.assertIn("Get-ChildItem -LiteralPath 'C:\\Program Files\\Adobe' -Directory", self.ps1)
        self.assertIn("-Recurse -Depth 3", self.ps1)
        self.assertIn("throw 'not installed'", self.ps1)
        self.assertIn("GetFileNameWithoutExtension", self.ps1)          # the process name of the window wait comes from the discovered exe
        self.assertIn("$R.exe_dir = Split-Path (Split-Path $c.exe -Parent) -Leaf", self.ps1)   # the directory NAME, never the full path
        self.assertNotRegex(self.ps1, r"\$R\.\w+ = \$found\b")

    def test_the_inventory_is_written_before_the_discovery(self):
        self.assertLess(self.ps1.index("Write-Inventory\n"), self.ps1.index("Find-AppExe $c.find"))
        self.assertIn("inventory.txt", self.ps1)
        self.assertIn("Select-Object -First", self.ps1)

    def test_a_missing_process_name_never_reaches_quit_app(self):
        self.assertIn("if ($c.proc) { Quit-App $c.proc }", self.ps1)

    def test_no_screenshot_route_in_the_launch_script(self):
        for bad in ("CopyFromScreen", "Bitmap", "PrintWindow", "Screenshot", "screenshot -", ".png", ".jpg"):
            self.assertNotIn(bad, self.ps1)

    def test_known_rows_keep_their_versioned_paths(self):
        for name in ("premiere", "lightroomlaunch", "dimension", "uxp", "bridge", "characteranimator", "audition"):
            self.assertIn("exe = 'C:\\Program Files\\Adobe\\", self.entry(name), name)

    def test_the_table_names_every_launch_row(self):
        for name in B.LAUNCH_ONLY:
            if name == "mediaencoder":
                continue
            self.entry(name)


class CaptureScript(unittest.TestCase):
    def setUp(self):
        self.ps1 = LAUNCH.read_text()

    def test_the_capture_runs_twice_after_the_hold_and_after_another_sixty_seconds(self):
        self.assertEqual(self.ps1.count("Add-UiText $c.proc $c.title"), 2)
        a, b = (m.start() for m in re.finditer(re.escape("Add-UiText $c.proc $c.title"), self.ps1))
        between = self.ps1[a:b]
        self.assertIn("Wait-For { $false } $c.proc $hold", between)
        self.assertIn("[Math]::Min(60,", between)
        self.assertIn("Wait-For { $false } $c.proc 30", self.ps1[:a])

    def test_the_capture_is_best_effort_and_never_throws_into_the_row(self):
        body = self.ps1[self.ps1.index("function Add-UiText"):]
        body = body[:body.index("\n}\n")]
        self.assertIn("try {", body)
        self.assertIn("catch", body)
        self.assertIn("$Script:UiErrors +=", self.ps1)
        self.assertIn("$R.uitext_error", self.ps1)

    def test_the_walk_has_a_hard_cap_the_element_bound_and_runs_off_the_main_thread(self):
        self.assertIn("[int]$maxEl = 3000", self.ps1)
        self.assertIn("[int]$capS = 60", self.ps1)
        self.assertIn("[powershell]::Create()", self.ps1)
        self.assertIn("BeginInvoke()", self.ps1)
        self.assertIn("BeginStop", self.ps1)
        self.assertIn("TreeWalker]::ControlViewWalker", self.ps1)

    def test_the_walk_covers_the_process_its_children_and_windows_by_title(self):
        self.assertIn("ParentProcessId", self.ps1)
        self.assertIn("IndexOf($title, [StringComparison]::OrdinalIgnoreCase)", self.ps1)

    def test_files_and_bounds(self):
        self.assertIn("uitext.txt", self.ps1)
        self.assertIn("uitext_hits.txt", self.ps1)
        self.assertIn("200000", self.ps1)
        self.assertIn("$R.ui_hits", self.ps1)
        self.assertIn("WriteAllLines", self.ps1)

    def test_the_scrub_is_one_function_and_every_written_line_goes_through_it(self):
        self.assertEqual(len(re.findall(r"(?m)^function Scrub-Text", self.ps1)), 1)
        for needle in ("Scrub-Text $_", "$line = Scrub-Text", "Scrub-Text ($_.Exception.Message)"):
            self.assertIn(needle, self.ps1)

    def test_the_scrub_patterns_equal_the_python_ones(self):
        m = re.search(r"\$Script:ScrubRes = @\((.*?)\n\)\n", self.ps1, re.S)
        self.assertTrue(m)
        self.assertEqual(tuple(re.findall(r"'([^']+)'", m.group(1))), B.SCRUB_RES)         # the PowerShell list is the source of the comparison
        self.assertIn("$Script:ScrubAfter = '%s'" % B.SCRUB_AFTER, self.ps1)
        self.assertIn("'<scrubbed>'", self.ps1)
        self.assertIn("'$1 <scrubbed>'", self.ps1)

    def test_the_hit_words_equal_the_python_ones(self):
        m = re.search(r"\$Script:HitWords = @\((.*?)\)\n", self.ps1, re.S)
        self.assertTrue(m)
        words = re.findall(r"'([^']+)'", m.group(1))
        self.assertEqual(tuple(words), B.HIT_WORDS)


class Words(unittest.TestCase):
    def test_the_requested_words_are_all_hit_words(self):
        for w in ("download", "install", "online", "model", "not available", "unavailable", "requires internet", "internet connection", "internet",
                  "sign in", "sign-in", "signin", "log in", "trial", "offline", "update", "connect", "licen", "subscription", "get started", "free",
                  "activate", "add-on", "addon", "plug-in", "plugin", "content", "library", "cloud", "account"):
            self.assertIn(w, B.HIT_WORDS)


class Scrub(unittest.TestCase):
    def test_email_guid_hex_key_and_after_keyword(self):
        s = B.scrub_text
        self.assertEqual(s("Text | jane.doe+x@example.co.uk | id"), "Text | <scrubbed> | id")
        self.assertEqual(s("a 3f2504e0-4f89-11d3-9a0c-0305e82c3301 b"), "a <scrubbed> b")
        self.assertEqual(s("token 0123456789abcdef0123 end"), "token <scrubbed> end")
        self.assertEqual(s("key ABCD-1234-EFGH-5678 ok"), "key <scrubbed> ok")
        self.assertEqual(s("key ABCDE-12345-FGHIJK-5678A-ZZZZ1 ok"), "key <scrubbed> ok")
        self.assertEqual(s("Signed in as Jane Doe (Pro)"), "Signed in as <scrubbed>")
        self.assertEqual(s("Text | Welcome back, Jane | x"), "Text | Welcome <scrubbed>")
        self.assertEqual(s("hello Jane"), "hello <scrubbed>")

    def test_ordinary_text_is_kept(self):
        for keep in ("Button | Download Models | dl", "Text | Install the add-on | a", "Text | Requires an internet connection", "Window | Adobe Substance 3D Painter | main",
                     "Text | 2026-11-20 | d", "Text | plug-in-free", "Text | Sign in required"):
            self.assertEqual(B.scrub_text(keep), keep)

    def test_a_multi_line_text_is_scrubbed_line_by_line_and_keeps_its_newline(self):
        self.assertEqual(B.scrub_text("a@b.co\nWelcome Jane\nplain\n"), "<scrubbed>\nWelcome <scrubbed>\nplain\n")
        self.assertEqual(B.scrub_text(""), "")


class FakeGuest:
    """Just enough of the guest transport for propimage_apps_b.run_check: files in a dict, the bound check of the pull command evaluated here."""
    user = "kfimage"

    def __init__(self, files, grown=()):
        self.files, self.asked, self.pulled, self.grown = files, [], [], grown       # `grown`: files that pass the size check and are bigger when pulled

    def ps(self, cmd):
        r = types.SimpleNamespace(returncode=0, stdout="", stderr="")
        if "ParseFile" in cmd:
            r.stdout = "parse-ok"
        elif "result.json'){'ready'}" in cmd:
            r.stdout = "ready"
        elif "Get-Item" in cmd:
            m = re.search(r"Test-Path '[^']*/([^'/]+)'\)\{\$f=Get-Item '[^']*'.*-le (\d+)", cmd)
            fn, limit = m.group(1), int(m.group(2))
            self.asked.append((fn, limit))
            if fn in self.files and (len(self.files[fn]) <= limit or fn in self.grown):
                r.stdout = "bounded"
        elif "started" in cmd or "New-ScheduledTask" in cmd:
            r.stdout = "started"
        return r

    def scp(self, src, dst, to_guest):
        if to_guest:
            return types.SimpleNamespace(returncode=0, stdout="", stderr="")
        name = src.rsplit("/", 1)[1]
        if name not in self.files:
            return types.SimpleNamespace(returncode=1, stdout="", stderr="missing")
        Path(dst).write_bytes(self.files[name])
        self.pulled.append(name)
        return types.SimpleNamespace(returncode=0, stdout="", stderr="")


def launch_result(app, **extra):
    r = {"app": app, "started": 1000.0, "ended": 1100.0, "error": None, "quit": "clean", "outputs": [], "main_window": True, "alive": True,
         "responding": True, "titles": ["w"], "private_mb": 300}
    r.update(extra)
    return json.dumps(r).encode()


class Pull(unittest.TestCase):
    def run_row(self, app, files, grown=()):
        guest = FakeGuest(files, grown)
        self.td = tempfile.TemporaryDirectory()
        self.addCleanup(self.td.cleanup)
        stub = lambda g, operation, **kw: (operation(), {"verdict": "PASS", "required": [], "peak": {}, "baseline": {}, "samples": 1, "reasons": []})  # noqa: E731
        with unittest.mock.patch.object(B.gpu_assert, "assert_gpu_used", stub):
            row = B.run_check(app, self.td.name, guest, sleep=lambda s: None)
        d = next((Path(self.td.name) / app).iterdir())
        return guest, row, d

    def test_the_ui_files_are_in_the_pull_list_of_launch_rows_only(self):
        for app in B.LAUNCH_ONLY:
            names = B.pull_names({"outputs": []}, app)
            for f in B.UI_FILES:
                self.assertIn(f, names, app)
        for app in ("photoshop", "acrobat", "gpudiag"):
            for f in B.UI_FILES:
                self.assertNotIn(f, B.pull_names({"outputs": []}, app), app)
        self.assertEqual(B.UI_FILES, ("uitext.txt", "uitext_hits.txt", "inventory.txt"))

    def test_the_ui_files_have_their_own_small_bound(self):
        for f in B.UI_FILES:
            self.assertEqual(B.pull_limit(f), B.MAX_UI_PULL)
        self.assertLessEqual(B.MAX_UI_PULL, 1 << 20)
        self.assertGreaterEqual(B.MAX_UI_PULL, 200 * 1000)
        self.assertEqual(B.pull_limit("sniffer.txt"), B.MAX_PULL)

    def test_run_check_pulls_the_ui_files_and_scrubs_them_on_the_host(self):
        files = {"result.json": launch_result("bridge"), "steps.log": b"x",
                 "uitext.txt": "Text | Signed in as jane@example.com | a\nButton | Download | b\n".encode(),
                 "uitext_hits.txt": b"Text | Key ABCD-1234-EFGH-5678 | x\n", "inventory.txt": b"root: Adobe\n"}
        guest, row, d = self.run_row("bridge", files)
        self.assertTrue(row["ok"], row)
        self.assertEqual(row["status"], "PASS_LIMITED")
        for f in B.UI_FILES:
            self.assertTrue((d / f).is_file(), f)
        text = (d / "uitext.txt").read_text()
        self.assertNotIn("jane", text)
        self.assertIn("Signed in as <scrubbed>", text)
        self.assertIn("Button | Download | b", text)
        self.assertNotIn("ABCD", (d / "uitext_hits.txt").read_text())
        self.assertEqual((d / "inventory.txt").read_text(), "root: Adobe\n")

    def test_an_oversized_ui_file_is_not_pulled(self):
        files = {"result.json": launch_result("bridge"), "uitext.txt": b"x" * (B.MAX_UI_PULL + 1), "uitext_hits.txt": b"Text | a | b\n"}
        guest, row, d = self.run_row("bridge", files)
        self.assertNotIn("uitext.txt", guest.pulled)
        self.assertFalse((d / "uitext.txt").exists())
        self.assertIn("uitext_hits.txt", guest.pulled)
        self.assertIn(("uitext.txt", B.MAX_UI_PULL), guest.asked)
        self.assertTrue(row["ok"], row)         # the capture never changes a verdict

    def test_a_missing_ui_file_is_not_an_error(self):
        guest, row, d = self.run_row("bridge", {"result.json": launch_result("bridge")})
        self.assertTrue(row["ok"], row)
        self.assertNotIn("uitext.txt", guest.pulled)
        for f in B.UI_FILES:
            self.assertFalse((d / f).exists())

    def test_an_unsafe_name_in_the_guest_output_list_is_not_pulled(self):
        files = {"result.json": launch_result("bridge", outputs=[{"file": "../evil.txt", "bytes": 1, "modified": 1050.0}, {"file": "a b.txt"}]),
                 "../evil.txt": b"x", "a b.txt": b"x"}
        guest, row, d = self.run_row("bridge", files)
        self.assertEqual([n for n, _ in guest.asked if n in ("../evil.txt", "a b.txt")], [])
        self.assertEqual({n for n, _ in guest.asked if n.endswith("txt")} & {"../evil.txt", "a b.txt"}, set())

    def test_scrub_file_deletes_a_file_it_cannot_scrub(self):
        with tempfile.TemporaryDirectory() as td:
            p = Path(td) / "uitext.txt"
            p.write_bytes(b"x" * (B.MAX_UI_PULL + 5))
            B.scrub_file(p)
            self.assertFalse(p.exists())
            p.write_bytes(b"\xef\xbb\xbfText | a@b.co | \xff\n")
            B.scrub_file(p)
            self.assertEqual(p.read_bytes(), "Text | <scrubbed> | \ufffd\n".encode())


    def test_a_row_that_is_not_a_launch_row_is_never_asked_for_the_ui_files(self):
        files = {"result.json": launch_result("gpudiag"), "uitext.txt": b"x", "uitext_hits.txt": b"x", "inventory.txt": b"x"}
        guest, row, d = self.run_row("gpudiag", files)
        self.assertEqual([n for n, _ in guest.asked if n in B.UI_FILES], [])

    def test_a_file_that_grew_after_the_size_check_is_deleted(self):
        files = {"result.json": launch_result("bridge"), "uitext.txt": b"Text | a@b.co | x\n" * 40000, "uitext_hits.txt": b"Text | Download | x\n"}
        guest, row, d = self.run_row("bridge", files, grown=("uitext.txt",))
        self.assertIn("uitext.txt", guest.pulled)
        self.assertFalse((d / "uitext.txt").exists())
        self.assertTrue((d / "uitext_hits.txt").is_file())
        self.assertTrue(row["ok"], row)

    def test_every_ui_file_is_scrubbed_including_the_inventory(self):
        mail = b"root: a@b.co\nText | c@d.org | key ABCD-1234-EFGH-5678\n"
        files = {"result.json": launch_result("bridge"), "uitext.txt": mail, "uitext_hits.txt": mail, "inventory.txt": mail}
        guest, row, d = self.run_row("bridge", files)
        for f in B.UI_FILES:
            text = (d / f).read_text()
            self.assertEqual(text.count("<scrubbed>"), 3, f)
            self.assertNotIn("@", text, f)


class GradeLaunch(unittest.TestCase):
    def setUp(self):
        self.td = tempfile.TemporaryDirectory()
        self.d = Path(self.td.name)
        self.addCleanup(self.td.cleanup)

    def put(self, **extra):
        (self.d / "result.json").write_bytes(launch_result("audition", **extra))

    def test_ui_hits_are_counted_from_the_pulled_file_and_reported(self):
        self.put(ui_hits=99)
        (self.d / "uitext_hits.txt").write_text("a\nb\n\nc\n")
        row = B.verdict("audition", "x", self.d, None)
        self.assertTrue(row["ok"], row)
        self.assertEqual(row["ui_hits"], 3)
        self.assertIn("ui_hits=3", row["signal"])
        self.assertEqual((row["status"], row["limited"], row["evidence"]), ("PASS_LIMITED", True, "UIA"))

    def test_blank_lines_are_not_counted(self):
        self.put()
        (self.d / "uitext_hits.txt").write_text("a\n\n   \nb\n")
        self.assertEqual(B.verdict("audition", "x", self.d, None)["ui_hits"], 2)

    def test_without_the_file_the_guest_count_is_used_and_without_both_it_is_zero(self):
        self.put(ui_hits=4)
        self.assertEqual(B.verdict("audition", "x", self.d, None)["ui_hits"], 4)
        self.put()
        row = B.verdict("audition", "x", self.d, None)
        self.assertEqual(row["ui_hits"], 0)
        self.assertTrue(row["ok"])
        self.put(ui_hits="many")
        self.assertEqual(B.verdict("audition", "x", self.d, None)["ui_hits"], 0)

    def test_the_capture_never_changes_a_verdict(self):
        self.put(uitext_error="walk cap reached", ui_hits=7)
        (self.d / "uitext_hits.txt").write_text("a\n" * 7)
        row = B.verdict("audition", "x", self.d, None)
        self.assertTrue(row["ok"] and row["limited"], row)
        self.put(main_window=False)
        self.assertFalse(B.verdict("audition", "x", self.d, None)["ok"])
        self.put(alive=False)
        self.assertFalse(B.verdict("audition", "x", self.d, None)["ok"])
        self.put(quit="forced")
        self.assertFalse(B.verdict("audition", "x", self.d, None)["ok"])
        self.put(error="not installed")
        row = B.verdict("audition", "x", self.d, None)
        self.assertFalse(row["ok"])
        self.assertEqual(row["error"], "not installed")

    def test_a_non_launch_row_has_no_ui_hits_field(self):
        (self.d / "result.json").write_bytes(json.dumps({"app": "gpudiag", "started": 1.0, "ended": 9.0, "error": "x", "outputs": []}).encode())
        self.assertNotIn("ui_hits", B.verdict("gpudiag", "x", self.d, None))


if __name__ == "__main__":
    unittest.main()
