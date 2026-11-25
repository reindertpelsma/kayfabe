# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Harness of the DirectX-Graphics-Samples rows (STATUS: DESIGN-ONLY, 2026-11-17; the decision logic is GPU-free and tested with fakes,
the Win32 layer (ctypes) never ran on Windows).

The windowed samples have no command line besides -warp (CHECKED: DXSample::ParseCommandLineArgs; the harness never passes it) and end when
their window is closed (Win32Application::Run returns the WM_QUIT wParam: 0). So the harness starts the exe, waits for its top-level window,
lets it run, posts keys (ExecuteIndirect toggles culling with SPACE, ReservedResources steps the mip with UP/DOWN and puts "Mip Level: N" in
the title, HDR switches the swap-chain format with Page Up/Down, CHECKED in the pinned sources), takes WINDOW-ONLY captures (PrintWindow of that
one HWND; never a screenshot of the desktop), posts WM_CLOSE and asserts the exit code. The GPU-use proof (host census: process-tree GPU
Engine counters 3D and Compute) is made by the matrix supervisor around this harness (guest/kf_run_app.ps1); engines_all / engines_any in the
row only tell the matrix what to require (apps.json proof, proof_all).

Console samples (HelloWorkGraphs) are run with a timeout; the harness parses the UAV values the sample prints.
"""
import hashlib
import re
import sys
import time

from kfopen_common import OpenRowError, check_line, say

VK = {"SPACE": 0x20, "PRIOR": 0x21, "NEXT": 0x22, "UP": 0x26, "DOWN": 0x28, "H": 0x48, "U": 0x55}
MIN_UNIQUE_COLOURS = 8
SAMPLE_PIXELS = 4096
MIN_WINDOW_PX = 64
WINDOW_WAIT_S = 90
CLOSE_WAIT_S = 20
KEY_SETTLE_S = 0.4


# ------------------------------------------------------------------------------------------------ pixels
def analyze_pixels(bgra, width, height):
    """Window capture bytes (BGRA, top-down) -> {sha256, unique, nonblank}. The hash is over every byte; blankness is judged on ~4096 sampled
    pixels (alpha ignored): a capture of one colour (a black or cleared window) is blank."""
    if width < MIN_WINDOW_PX or height < MIN_WINDOW_PX or len(bgra) != width * height * 4:
        raise OpenRowError("CAPTURE_SHAPE %dx%d with %d bytes" % (width, height, len(bgra)))
    npix = width * height
    step = max(1, npix // SAMPLE_PIXELS)
    seen = set()
    for i in range(0, npix, step):
        seen.add(bgra[i * 4:i * 4 + 3])
        if len(seen) > 4096:
            break
    return {"sha256": hashlib.sha256(bgra).hexdigest(), "unique": len(seen), "nonblank": len(seen) >= MIN_UNIQUE_COLOURS,
            "width": width, "height": height}


# ------------------------------------------------------------------------------------------------ the title model of ReservedResources
def expected_mip_titles(first_mip, keys, gap_ok=True):
    """Titles expected after each key given the first title N0 (the sample starts at the least detailed mip, N0 = levels-1, and UP moves to
    more detail down to 0, DOWN back up to N0). Returns the list of expected mip numbers, one per key; a key that cannot move keeps the number."""
    cur, out = first_mip, []
    for vk in keys:
        if vk == "UP":
            cur = max(cur - 1, 0)
        elif vk == "DOWN":
            cur = min(cur + 1, first_mip)
        out.append(cur)
    return out


def expand_keys(keys):
    """[{"vk": "UP", "repeat": 8}, ...] -> ["UP"]*8 + ... ; each item may carry after_s (a wait before the first press)."""
    out = []
    for k in keys:
        if k["vk"] not in VK:
            raise OpenRowError("unknown key %s" % k["vk"])
        out += [k["vk"]] * int(k.get("repeat", 1))
    return out


def mip_from_title(title, rx):
    m = re.search(rx, title or "")
    return int(m.group(1)) if m else None


# ------------------------------------------------------------------------------------------------ the windowed run
class Clock:
    def __init__(self):
        self.now, self.sleep = time.time, time.sleep


def run_window_sample(spec, popen, win, clock=None, log=print):
    """Drive one windowed sample. `popen` is a started subprocess-like (pid, poll(), wait(timeout), kill()); `win` the Win32 layer
    (find_window(pid), title(h), post_key(h, vk), post_close(h), capture(h) -> (w, h, bytes)). Returns the facts dict judged by judge_window()."""
    clock = clock or Clock()
    t0 = clock.now()
    facts = {"window_found": False, "early_exit_rc": None, "titles": [], "captures": [], "keys": [], "close": {}}
    hwnd = None
    while clock.now() - t0 < WINDOW_WAIT_S:
        rc = popen.poll()
        if rc is not None:
            facts["early_exit_rc"] = rc
            return facts
        hwnd = win.find_window(popen.pid)
        if hwnd:
            break
        clock.sleep(0.5)
    if not hwnd:
        popen.kill()
        return facts
    facts["window_found"], facts["window_secs"] = True, round(clock.now() - t0, 1)
    run_s = float(spec["seconds"])
    t_run = clock.now()
    seq = expand_keys(spec.get("keys", []))
    cap_times = [min(5.0, run_s / 4), run_s / 2]
    caps_done = 0
    # settle, then the first title; a row that checks titles waits (up to 20 s) for the first frame to set it
    clock.sleep(2.0)
    first = win.title(hwnd)
    t_title = clock.now()
    while spec.get("title_regex") and mip_from_title(first, spec["title_regex"]) is None and clock.now() - t_title < 20.0 and popen.poll() is None:
        clock.sleep(0.5)
        first = win.title(hwnd)
    facts["titles"].append(("start", first))
    for vk in seq:
        if popen.poll() is not None:
            facts["early_exit_rc"] = popen.poll()
            return facts
        win.post_key(hwnd, VK[vk])
        facts["keys"].append(vk)
        clock.sleep(KEY_SETTLE_S)
        facts["titles"].append((vk, win.title(hwnd)))
    while clock.now() - t_run < run_s:
        rc = popen.poll()
        if rc is not None:
            facts["early_exit_rc"] = rc
            return facts
        if caps_done < len(cap_times) and clock.now() - t_run >= cap_times[caps_done]:
            w, h, px = win.capture(hwnd)
            a = analyze_pixels(px, w, h)
            a["t"] = round(clock.now() - t_run, 1)
            facts["captures"].append(a)
            caps_done += 1
        clock.sleep(0.5)
    while caps_done < 2:                                    # a short run still gets two captures, one second apart
        w, h, px = win.capture(hwnd)
        a = analyze_pixels(px, w, h)
        a["t"] = round(clock.now() - t_run, 1)
        facts["captures"].append(a)
        caps_done += 1
        clock.sleep(1.0)
    t_close = clock.now()
    win.post_close(hwnd)
    exited = False
    while clock.now() - t_close < CLOSE_WAIT_S:
        if popen.poll() is not None:
            exited = True
            break
        clock.sleep(0.25)
    if not exited:
        popen.kill()
    facts["close"] = {"exited": exited, "rc": popen.poll(), "secs": round(clock.now() - t_close, 1)}
    return facts


def judge_window(spec, facts):
    """Named checks of a windowed run: [(name, ok, why)]. All must hold for a PASS; the GPU census proof is the supervisor's."""
    checks = []
    early = facts.get("early_exit_rc")
    checks.append(("sample_started_window", bool(facts.get("window_found")) and early is None,
                   ("exited rc=%s before it drew a window" % early) if early is not None else "no top-level window within %d s" % WINDOW_WAIT_S))
    if not facts.get("window_found") or early is not None:
        return checks
    caps = facts.get("captures", [])
    checks.append(("window_captures", len(caps) >= 2, "%d window captures (need 2)" % len(caps)))
    checks.append(("screenshot_nonblank", bool(caps) and all(c["nonblank"] for c in caps),
                   "blank window capture (%s unique colours)" % ",".join(str(c["unique"]) for c in caps)))
    if spec.get("frames_advance") and len(caps) >= 2:
        checks.append(("frames_advance", len({c["sha256"] for c in caps}) >= 2, "two captures %.1f s apart are identical: the animation did not advance" % (caps[-1]["t"] - caps[0]["t"])))
    if spec.get("title_regex"):
        mips = [mip_from_title(t, spec["title_regex"]) for _, t in facts.get("titles", [])]
        if not mips or mips[0] is None:
            checks.append(("title_sequence", False, 'no "%s" title: the tiled-resource path did not start (tier unsupported or stuck)' % spec["title_regex"]))
        else:
            want = expected_mip_titles(mips[0], facts["keys"])
            got = mips[1:]
            checks.append(("title_sequence", got == want, "mip titles %s, expected %s" % (got, want)))
    close = facts.get("close", {})
    checks.append(("close_graceful", bool(close.get("exited")), "window did not end the process %d s after WM_CLOSE" % CLOSE_WAIT_S))
    checks.append(("exit_code_zero", close.get("rc") == 0, "exit code %s (WM_QUIT wParam 0 expected)" % close.get("rc")))
    return checks


def report(row_id, spec, checks, facts, stream=None):
    for name, ok, why in checks:
        check_line(name, ok, why)
    for c in facts.get("captures", []):
        say("WINDOW_HASH", "%s t=%s unique=%d nonblank=%s %dx%d" % (c["sha256"], c["t"], c["unique"], c["nonblank"], c["width"], c["height"]))
    for k, t in facts.get("titles", []):
        say("TITLE", "%s %s" % (k, t))
    ok = all(c[1] for c in checks)
    if ok:
        say("VERDICT", "PASS row=%s keys=%d" % (row_id, len(facts.get("keys", []))))
    else:
        say("VERDICT", "FAIL row=%s failed=%s" % (row_id, ",".join(c[0] for c in checks if not c[1])))
    return 0 if ok else 1


# ------------------------------------------------------------------------------------------------ console sample (work graphs)
def parse_uav(text, rx):
    return [(int(i), int(v, 16)) for i, v in re.findall(rx, text)]


def judge_console(spec, rc, text, control=None):
    """(checks, status, uav). status is "ok", "notrun" (the device reports no work graph tier) or "fail"."""
    if spec.get("not_supported_regex") and re.search(spec["not_supported_regex"], text):
        return [], "notrun", []
    uav = parse_uav(text, spec["uav_regex"])
    checks = [("exit_code_zero", rc == 0, "exit code %s" % rc),
              ("execution_complete", re.search(spec["ok_regex"], text) is not None, "no '%s' line" % spec["ok_regex"]),
              ("uav_lines", len(uav) == spec["uav_lines"], "%d UAV lines, expected %d" % (len(uav), spec["uav_lines"])),
              ("uav_written", any(v for _, v in uav), "every printed UAV value is zero: the graph wrote nothing")]
    if control is not None:
        checks.append(("equals_control", [list(x) for x in uav] == [list(x) for x in control], "UAV values differ from the native control"))
    status = "ok" if all(c[1] for c in checks) else "fail"
    return checks, status, uav


# ------------------------------------------------------------------------------------------------ Win32 layer (never ran; ctypes)
class Win32:
    """Thin ctypes layer. Window-only capture uses PrintWindow(PW_RENDERFULLCONTENT) on the sample's own HWND; there is no desktop capture."""

    def __init__(self):
        import ctypes
        from ctypes import wintypes
        self.ct, self.wt = ctypes, wintypes
        self.u, self.g = ctypes.windll.user32, ctypes.windll.gdi32
        self._prototypes()
        try:
            self.u.SetProcessDPIAware()
        except OSError:
            pass

    def _prototypes(self):
        """Explicit argument and return types: ctypes would pass handles as 32-bit ints otherwise (the usual 64-bit Windows trap)."""
        ct, wt = self.ct, self.wt
        H, I, U, L, B, P = wt.HANDLE, ct.c_int, wt.UINT, wt.LPARAM, wt.BOOL, ct.c_void_p
        for lib, name, args, res in (
                (self.u, "GetWindowThreadProcessId", [wt.HWND, ct.POINTER(wt.DWORD)], wt.DWORD), (self.u, "IsWindowVisible", [wt.HWND], B),
                (self.u, "GetWindow", [wt.HWND, U], wt.HWND), (self.u, "GetWindowTextLengthW", [wt.HWND], I),
                (self.u, "GetWindowTextW", [wt.HWND, wt.LPWSTR, I], I), (self.u, "PostMessageW", [wt.HWND, U, wt.WPARAM, L], B),
                (self.u, "GetWindowRect", [wt.HWND, ct.POINTER(wt.RECT)], B), (self.u, "GetDC", [wt.HWND], H), (self.u, "ReleaseDC", [wt.HWND, H], I),
                (self.u, "PrintWindow", [wt.HWND, H, U], B), (self.g, "CreateCompatibleDC", [H], H), (self.g, "CreateCompatibleBitmap", [H, I, I], H),
                (self.g, "SelectObject", [H, H], H), (self.g, "DeleteObject", [H], B), (self.g, "DeleteDC", [H], B),
                (self.g, "GetDIBits", [H, H, U, U, P, P, U], I)):
            fn = getattr(lib, name)
            fn.argtypes, fn.restype = args, res

    def find_window(self, pid):
        ct, wt = self.ct, self.wt
        found = []
        proto = ct.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)

        def cb(hwnd, _):
            p = wt.DWORD()
            self.u.GetWindowThreadProcessId(hwnd, ct.byref(p))
            if p.value == pid and self.u.IsWindowVisible(hwnd) and not self.u.GetWindow(hwnd, 4) and self.u.GetWindowTextLengthW(hwnd) > 0:
                found.append(hwnd)
            return True
        self.u.EnumWindows(proto(cb), 0)
        return found[0] if found else None

    def title(self, hwnd):
        buf = self.ct.create_unicode_buffer(256)
        self.u.GetWindowTextW(hwnd, buf, 256)
        return buf.value

    def post_key(self, hwnd, vk):
        self.u.PostMessageW(hwnd, 0x0100, vk, 0)            # WM_KEYDOWN: the samples read it in their WndProc, no focus needed
        self.u.PostMessageW(hwnd, 0x0101, vk, 0xC0000001)   # WM_KEYUP

    def post_close(self, hwnd):
        self.u.PostMessageW(hwnd, 0x0010, 0, 0)             # WM_CLOSE

    def capture(self, hwnd):
        ct, wt = self.ct, self.wt
        r = wt.RECT()
        self.u.GetWindowRect(hwnd, ct.byref(r))
        w, h = r.right - r.left, r.bottom - r.top
        screen_dc = self.u.GetDC(0)                         # only a source of a compatible DC: nothing is read from the desktop
        mem = self.g.CreateCompatibleDC(screen_dc)
        bmp = self.g.CreateCompatibleBitmap(screen_dc, w, h)
        old = self.g.SelectObject(mem, bmp)
        try:
            if not self.u.PrintWindow(hwnd, mem, 2):        # PW_RENDERFULLCONTENT
                raise OpenRowError("CAPTURE_FAILED PrintWindow returned 0")

            class BIH(ct.Structure):
                _fields_ = [("biSize", wt.DWORD), ("biWidth", wt.LONG), ("biHeight", wt.LONG), ("biPlanes", wt.WORD), ("biBitCount", wt.WORD),
                            ("biCompression", wt.DWORD), ("biSizeImage", wt.DWORD), ("biXPelsPerMeter", wt.LONG), ("biYPelsPerMeter", wt.LONG),
                            ("biClrUsed", wt.DWORD), ("biClrImportant", wt.DWORD)]
            bih = BIH(ct.sizeof(BIH), w, -h, 1, 32, 0, 0, 0, 0, 0, 0)
            buf = (ct.c_ubyte * (w * h * 4))()
            if not self.g.GetDIBits(mem, bmp, 0, h, ct.byref(buf), ct.byref(bih), 0):
                raise OpenRowError("CAPTURE_FAILED GetDIBits returned 0")
            return w, h, bytes(buf)
        finally:
            self.g.SelectObject(mem, old)
            self.g.DeleteObject(bmp)
            self.g.DeleteDC(mem)
            self.u.ReleaseDC(0, screen_dc)


def not_windows():
    return not sys.platform.startswith("win")
