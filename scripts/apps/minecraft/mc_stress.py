#!/usr/bin/env python3
"""Repeatable Minecraft GPU stress (offline, no credentials, no extra tools).

  pregen : official server jar, fixed seed, default generator, forceload tiles -> pristine world
  run    : N times: fresh copy of the world, dedicated server (online-mode=false) on localhost, client joins via
           --quickPlayMultiplayer, server console drives a deterministic camera path (/tp) and a fixed entity set,
           F3 overlay (injected through /dev/uinput) is screenshotted and its FPS number decoded from the PNG.
Run as root (needs /dev/uinput, dmesg); the client and screenshots run as user ubuntu. python3 -B -I.
"""
import argparse, importlib.util, json, math, os, re, shutil, signal, statistics, struct, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("mc_trial", f"{HERE}/mc_trial.py")
mt = importlib.util.module_from_spec(spec); spec.loader.exec_module(mt)

SEED = "kayfabe-2026"
PORT = 25599
XA_GLOB = "/run/user/1000/.mutter-Xwaylandauth."
UENV = lambda: ["runuser", "-u", "ubuntu", "--", "env", "DISPLAY=:0",
                "XAUTHORITY=" + next(os.path.join("/run/user/1000", f) for f in os.listdir("/run/user/1000") if f.startswith(".mutter-Xwaylandauth.")),
                "XDG_RUNTIME_DIR=/run/user/1000"]


class Server:
    def __init__(self, root, props):
        self.dir = f"{root}/server"; self.lines = []
        open(f"{self.dir}/eula.txt", "w").write("eula=true\n")
        open(f"{self.dir}/server.properties", "w").write("".join(f"{k}={v}\n" for k, v in props.items()))
        self.p = subprocess.Popen(["java", "-Xmx4G", "-jar", "server.jar", "nogui"], cwd=self.dir, stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, text=True, bufsize=1, start_new_session=True)
        threading.Thread(target=self._rd, daemon=True).start()
    def _rd(self):
        for l in self.p.stdout: self.lines.append(l.rstrip())
    def cmd(self, c):
        self.p.stdin.write(c + "\n"); self.p.stdin.flush()
    def wait(self, rx, timeout, start=0):
        t0 = time.time()
        while time.time() - t0 < timeout:
            for i in range(start, len(self.lines)):
                if re.search(rx, self.lines[i]): return i
            time.sleep(0.2)
        return None
    def stop(self):
        if self.p.poll() is None:
            try: self.cmd("stop"); self.p.wait(60)
            except Exception: pass
        if self.p.poll() is None: os.killpg(self.p.pid, signal.SIGKILL)


def props(view):
    return {"level-type": "minecraft\\:normal", "level-seed": SEED, "level-name": "world", "online-mode": "false",
            "server-port": PORT, "spawn-protection": 0, "max-players": 1, "gamemode": "creative", "difficulty": "peaceful",
            "view-distance": view, "simulation-distance": 16, "pause-when-empty-seconds": -1, "sync-chunk-writes": "false",
            "generate-structures": "true", "spawn-monsters": "false"}


def cmd_pregen(a):
    t0 = time.time()
    shutil.rmtree(f"{a.root}/server/world", ignore_errors=True)
    s = Server(a.root, props(32))
    if s.wait(r"Done \(", 240) is None: sys.exit("pregen: server not up")
    n = a.tiles  # tiles per axis, 16x16 chunks each, centred on 0
    lo = -(n // 2) * 16
    total = 0
    for i in range(n):
        for j in range(n):
            cx, cz = lo + i * 16, lo + j * 16
            x1, z1, x2, z2 = cx * 16, cz * 16, (cx + 15) * 16 + 15, (cz + 15) * 16 + 15
            s.cmd(f"forceload add {x1} {z1} {x2} {z2}")
            ok = False
            for _ in range(180):
                k = len(s.lines); s.cmd(f"execute if loaded {x2} 64 {z2}")
                if s.wait(r"Test passed", 1.5, k) is not None: ok = True; break
            if not ok: s.stop(); sys.exit(f"pregen: tile {i},{j} never loaded")
            s.cmd(f"forceload remove {x1} {z1} {x2} {z2}"); total += 256
    s.cmd("forceload remove all"); s.cmd("save-all flush"); s.wait(r"Saved the game", 120); s.stop()
    shutil.rmtree(f"{a.root}/world_pristine", ignore_errors=True)
    shutil.copytree(f"{s.dir}/world", f"{a.root}/world_pristine")
    sz = sum(os.path.getsize(os.path.join(d, f)) for d, _, fs in os.walk(f"{a.root}/world_pristine") for f in fs)
    print(f"pregen ok chunks={total} bytes={sz} secs={time.time()-t0:.0f}")


def uinput_press(code):
    import fcntl
    fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK)
    fcntl.ioctl(fd, 0x40045564, 1); fcntl.ioctl(fd, 0x40045565, code)
    os.write(fd, struct.pack("80sHHHHi", b"kf-mc-kbd", 3, 0x1234, 0x5678, 1, 0) + b"\0" * (4 * 64 * 4))
    fcntl.ioctl(fd, 0x5501); time.sleep(1.5)
    ev = lambda t, c, v: os.write(fd, struct.pack("llHHi", 0, 0, t, c, v))
    ev(1, code, 1); ev(0, 0, 0); time.sleep(0.1); ev(1, code, 0); ev(0, 0, 0); time.sleep(0.5)
    fcntl.ioctl(fd, 0x5502); os.close(fd)


def grab(path):
    tree = subprocess.run(UENV() + ["xwininfo", "-root", "-tree"], capture_output=True, text=True).stdout
    w = re.search(r'(0x[0-9a-f]+) "Minecraft[^"]*": \("Minecraft', tree)
    if not w: return False
    subprocess.run(" ".join(UENV()) + f" sh -c 'xwd -silent -id {w.group(1)}' | magick xwd:- {path}", shell=True, capture_output=True)
    return os.path.exists(path) and os.path.getsize(path) > 1000


class Fps:
    """Decode the leading 'NNN fps' number of the F3 overlay using the game's own font atlas (ascii.png)."""
    def __init__(self, root, ver):
        from PIL import Image
        self.Image = Image
        import zipfile, io
        z = zipfile.ZipFile(f"{root}/versions/{ver}/{ver}.jar")  # the font atlas ships inside the client jar
        im = Image.open(io.BytesIO(z.read("assets/minecraft/textures/font/ascii.png"))).convert("RGBA")
        self.cell = im.width // 16
        self.tpl = {}
        for d in "0123456789":
            c = ord(d); ox, oy = (c % 16) * self.cell, (c // 16) * self.cell
            g = [[im.getpixel((ox + x, oy + y))[3] > 0 for x in range(self.cell)] for y in range(self.cell)]
            rows = [y for y in range(self.cell) if any(g[y])]
            self.tpl[d] = (g, rows[0])
    def read(self, png, scale=2):
        im = self.Image.open(png).convert("RGB"); W, H = im.size
        px = im.load(); s = scale; c = self.cell
        lit = lambda x, y: all(v > 200 for v in px[x, y])
        best = (0, None)
        for y0 in range(0, 70):
            for x0 in range(0, 14):
                out = ""
                for k in range(6):
                    cx = x0 + k * 6 * s  # digit advance: 5 px glyph + 1 px gap
                    hit = None
                    for d, (g, top) in self.tpl.items():
                        ok = tot = 0
                        for y in range(c):
                            for x in range(5):
                                X, Y = cx + x * s + s // 2, y0 + (y - top) * s + s // 2
                                if 0 <= X < W and 0 <= Y < H:
                                    tot += 1; ok += (lit(X, Y) == g[y][x])
                        if tot and ok / tot > 0.97: hit = d; break
                    if hit is None: break
                    out += hit
                if len(out) > best[0]: best = (len(out), out)
            if best[0] >= 2: break
        return int(best[1]) if best[1] else None


def cmd_run(a):
    ver = a.version; root = a.root
    d = mt.vjson(root, ver)
    fps = Fps(root, ver)
    results = []
    for run in range(a.runs):
        r = dict(run=run + 1)
        try:
            results.append(one_run(a, d, fps, r))
        except Exception as e:  # a harness failure is a failed run, never silently skipped
            r["error"] = repr(e); r["pass"] = False; results.append(r)
        print(json.dumps(r), flush=True)
    meds = [r["fps_median"] for r in results if r.get("fps_median")]
    ok = len(results) == a.runs and all(r["pass"] for r in results)
    print("MC_STRESS %s runs=%d fps_medians=%s" % ("PASS" if ok else "FAIL", len(results), meds))
    sys.exit(0 if ok else 1)


def one_run(a, d, fps, r):
    root = a.root
    shutil.rmtree(f"{root}/server/world", ignore_errors=True)
    shutil.copytree(f"{root}/world_pristine", f"{root}/server/world")
    gd = f"{root}/game"
    for sub in ("logs", "crash-reports", "screenshots", "shots"):
        shutil.rmtree(f"{gd}/{sub}", ignore_errors=True)
    os.makedirs(f"{gd}/shots", exist_ok=True)
    open(f"{gd}/options.txt", "w").write(
        f"renderDistance:{a.render}\nsimulationDistance:{a.sim}\ngraphicsMode:1\nao:true\nmaxFps:260\nenableVsync:false\n"
        "guiScale:2\nbiomeBlendRadius:2\nentityShadows:true\nparticles:0\ncloudRenderMode:\"fancy\"\nmipmapLevels:4\n"
        "onboardAccessibility:false\ntutorialStep:none\npauseOnLostFocus:false\nskipMultiplayerWarning:true\n"
        "joinedFirstServer:true\nentityDistanceScaling:1.0\ninactivityFpsLimit:\"afk\"\n")
    subprocess.run(["chown", "-R", "ubuntu:ubuntu", gd, f"{root}/assets"], check=False)
    dm0 = len(subprocess.run(["dmesg"], capture_output=True, text=True).stdout.splitlines())
    srv = Server(root, props(a.render))
    smi = None; p = None
    try:
        if srv.wait(r"Done \(", 180) is None: raise RuntimeError("server not up")
        for c in ("gamerule doDaylightCycle false", "gamerule doMobSpawning false", "gamerule doWeatherCycle false",
                  "time set noon", "weather clear", "gamerule sendCommandFeedback false", "gamerule logAdminCommands false"):
            srv.cmd(c)
        a.mp = f"localhost:{PORT}"; a.w, a.h = a.width, a.height
        jvm, game, _ = mt.build_cmd(a, d)
        cmd = UENV() + ["java", "-Xmx4G"] + jvm + [d["mainClass"]] + game
        smi = subprocess.Popen(["nvidia-smi", "--query-gpu=utilization.gpu,memory.used,clocks.sm,power.draw", "--format=csv,noheader",
                                "-l", "1"], stdout=open(f"{root}/smi_{r['run']}.csv", "w"), start_new_session=True)
        t0 = time.time()
        p = subprocess.Popen(cmd, cwd=gd, stdout=open(f"{root}/client_{r['run']}.log", "w"), stderr=subprocess.STDOUT, start_new_session=True)
        if srv.wait(r"Dev joined the game", a.join_timeout) is None: raise RuntimeError("client never joined")
        r["join_s"] = round(time.time() - t0, 1)
        # fixed entity set, deterministic positions on a ring around the camera path
        n_ent = 0; lcg = 12345
        for kind, cnt in (("minecraft:sheep", 60), ("minecraft:villager", 30), ("minecraft:cow", 30)):
            for _ in range(cnt):
                lcg = (lcg * 1103515245 + 12345) & 0x7fffffff; ang = (lcg % 3600) / 10.0
                lcg = (lcg * 1103515245 + 12345) & 0x7fffffff; rad = 20 + lcg % 120
                x, z = int(rad * math.cos(math.radians(ang))), int(rad * math.sin(math.radians(ang)))
                srv.cmd(f"execute positioned {x} 319 {z} positioned over world_surface run summon {kind} ~ ~ ~ {{Silent:1b,PersistenceRequired:1b}}")
                n_ent += 1
        r["entities"] = n_ent
        srv.cmd("gamemode creative Dev"); srv.cmd("effect give Dev minecraft:resistance infinite 255 true")
        srv.cmd("tp Dev 80 110 0 90 -10"); time.sleep(a.settle)
        uinput_press(61)  # F3: debug overlay
        # deterministic camera path: circle R=80 at y=110, period 25 s, yaw spins twice per lap, pitch oscillates
        samples = []; t_start = time.time(); next_shot = 0.0; k = 0
        while time.time() - t_start < a.duration:
            t = time.time() - t_start
            ang = 2 * math.pi * t / 25.0
            yaw = ((t / 25.0) * 720.0) % 360 - 180
            pitch = -10 + 25 * math.sin(2 * math.pi * t / 12.5)
            srv.cmd(f"tp Dev {80*math.cos(ang):.2f} 110 {80*math.sin(ang):.2f} {yaw:.1f} {pitch:.1f}")
            if t >= next_shot:
                f = f"{gd}/shots/s{k:03d}.png"
                if grab(f):
                    v = fps.read(f)
                    if v is not None: samples.append(v)
                    elif k == 0: shutil.copy(f, f"{root}/first_shot_{r['run']}.png")
                k += 1; next_shot += a.sample_every
            if p.poll() is not None: raise RuntimeError("client died during path")
            time.sleep(0.1)
        alive = p.poll() is None
        r["alive"] = alive; r["fps_samples"] = samples
        if samples: r["fps_median"] = statistics.median(samples); r["fps_min"] = min(samples); r["fps_max"] = max(samples)
        gi = subprocess.run(UENV() + ["glxinfo", "-B"], capture_output=True, text=True).stdout
        m = re.search(r"OpenGL renderer string: (.*)", gi)
        libs = sorted(set(re.findall(r"(libGLX_nvidia[^\s]*|libnvidia-glcore[^\s]*|swrast[^\s]*|libgallium[^\s]*)", open(f"/proc/{p.pid}/maps").read())))
        # p is the runuser wrapper; the JVM is its child
        kids = subprocess.run(["pgrep", "-P", str(p.pid)], capture_output=True, text=True).stdout.split()
        for kpid in kids:
            try: libs = sorted(set(libs) | set(re.findall(r"(libGLX_nvidia[^\s]*|libnvidia-glcore[^\s]*|swrast[^\s]*|libgallium[^\s]*)", open(f"/proc/{kpid}/maps").read())))
            except OSError: pass
        r["gl"] = (m.group(1) if m else "none"); r["libs"] = libs
        shutil.copy(f, f"{root}/last_shot_{r['run']}.png") if k and os.path.exists(f) else None
    finally:
        for proc in (p, smi):
            if proc and proc.poll() is None:
                os.killpg(proc.pid, signal.SIGTERM)
                try: proc.wait(20)
                except subprocess.TimeoutExpired: os.killpg(proc.pid, signal.SIGKILL)
        srv.stop()
    dm = subprocess.run(["dmesg"], capture_output=True, text=True).stdout.splitlines()[dm0:]
    r["xid"] = sum(1 for l in dm if "NVRM: Xid" in l)
    r["crash"] = os.path.isdir(f"{gd}/crash-reports") and bool(os.listdir(f"{gd}/crash-reports"))
    util = []
    try:
        util = [int(l.split("%")[0]) for l in open(f"{root}/smi_{r['run']}.csv") if "%" in l]
    except Exception: pass
    if util: r["gpu_util_median"] = statistics.median(util); r["gpu_util_max"] = max(util)
    gpu_real = "NVIDIA GeForce" in r.get("gl", "") and any("libnvidia-glcore" in x for x in r.get("libs", []))
    r["gpu_real"] = gpu_real
    band = r.get("fps_median") is not None and r["fps_median"] >= a.fps_min and len(r.get("fps_samples", [])) >= a.min_samples
    r["fps_band_ok"] = band
    r["pass"] = bool(r.get("alive") and not r["crash"] and r["xid"] == 0 and gpu_real and band)
    return r


ap = argparse.ArgumentParser()
ap.add_argument("cmd", choices=["pregen", "run"])
ap.add_argument("--root", required=True)
ap.add_argument("--version", default="1.21.8")
ap.add_argument("--tiles", type=int, default=5, help="pregen: tiles per axis (16x16 chunks each)")
ap.add_argument("--runs", type=int, default=3)
ap.add_argument("--duration", type=int, default=100)
ap.add_argument("--sample-every", type=float, default=5.0)
ap.add_argument("--settle", type=int, default=15)
ap.add_argument("--render", type=int, default=32)
ap.add_argument("--sim", type=int, default=16)
ap.add_argument("--width", type=int, default=1280)
ap.add_argument("--height", type=int, default=720)
ap.add_argument("--fps-min", type=int, default=25)
ap.add_argument("--min-samples", type=int, default=10)
ap.add_argument("--join-timeout", type=int, default=150)
if __name__ == "__main__":
    a = ap.parse_args()
    {"pregen": cmd_pregen, "run": cmd_run}[a.cmd](a)
