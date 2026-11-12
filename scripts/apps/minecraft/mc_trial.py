#!/usr/bin/env python3
"""Offline, credential-free Minecraft Java trial: fetch (SHA-1 verified, official Mojang URLs only),
make a world with the official server jar, launch the client, screenshot, assert, PASS/FAIL line.
Run with: python3 -B -I mc_trial.py <fetch|world|run> --root DIR [...]. No account data is ever used."""
import argparse, hashlib, json, os, platform, re, subprocess, sys, time, urllib.request, shutil, signal

MANIFEST = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"
OFFLINE_UUID = "00000000-0000-0000-0000-000000000001"  # fixed fake offline id
ALLOWED = ("https://piston-meta.mojang.com/", "https://piston-data.mojang.com/",
           "https://libraries.minecraft.net/", "https://resources.download.minecraft.net/")

def get(url):
    assert url.startswith(ALLOWED), url
    with urllib.request.urlopen(url, timeout=60) as r:
        return r.read()

def fetch_file(url, sha1, dest):
    if os.path.exists(dest) and hashlib.sha1(open(dest, "rb").read()).hexdigest() == sha1:
        return 0
    data = get(url)
    if hashlib.sha1(data).hexdigest() != sha1:
        sys.exit(f"SHA1 MISMATCH {url}")
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    open(dest, "wb").write(data)
    return len(data)

def vjson(root, ver):
    p = f"{root}/versions/{ver}/{ver}.json"
    return json.load(open(p))

def rule_ok(rules, feats=()):
    if not rules: return True
    ok = False
    for r in rules:
        m = True
        if "os" in r:
            o = r["os"]
            if o.get("name") not in (None, "linux"): m = False
            if "arch" in o and o["arch"] not in ("x86_64",): m = False
        if "features" in r and not all(feats.count(k) for k in r["features"]): m = False
        if m: ok = r["action"] == "allow"
    return ok

def press_key(keyname):
    """Send a key via XTest (ctypes, no extra tools) to the focused X11 window."""
    import ctypes
    x11 = ctypes.CDLL("libX11.so.6"); xt = ctypes.CDLL("libXtst.so.6")
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    d = x11.XOpenDisplay(None)
    if not d: return False
    ks = x11.XStringToKeysym(keyname.encode()); kc = x11.XKeysymToKeycode(ctypes.c_void_p(d), ks)
    xt.XTestFakeKeyEvent(ctypes.c_void_p(d), kc, 1, 0); x11.XFlush(ctypes.c_void_p(d)); time.sleep(0.1)
    xt.XTestFakeKeyEvent(ctypes.c_void_p(d), kc, 0, 0); x11.XFlush(ctypes.c_void_p(d)); time.sleep(0.5)
    x11.XCloseDisplay(ctypes.c_void_p(d)); return True

def cmd_fetch(a):
    t0 = time.time(); total = 0
    man = json.loads(get(MANIFEST))
    ent = next(v for v in man["versions"] if v["id"] == a.version)
    raw = get(ent["url"]); assert hashlib.sha1(raw).hexdigest() == ent["sha1"]
    os.makedirs(f"{a.root}/versions/{a.version}", exist_ok=True)
    open(f"{a.root}/versions/{a.version}/{a.version}.json", "wb").write(raw)
    d = json.loads(raw)
    c = d["downloads"]["client"]
    total += fetch_file(c["url"], c["sha1"], f"{a.root}/versions/{a.version}/{a.version}.jar")
    s = d["downloads"]["server"]
    total += fetch_file(s["url"], s["sha1"], f"{a.root}/server/server.jar")
    for l in d["libraries"]:
        if not rule_ok(l.get("rules")): continue
        art = l["downloads"]["artifact"]
        total += fetch_file(art["url"], art["sha1"], f"{a.root}/libraries/{art['path']}")
    ai = d["assetIndex"]
    idx = get(ai["url"]); assert hashlib.sha1(idx).hexdigest() == ai["sha1"]
    os.makedirs(f"{a.root}/assets/indexes", exist_ok=True)
    open(f"{a.root}/assets/indexes/{ai['id']}.json", "wb").write(idx)
    objs = json.loads(idx)["objects"]
    for i, (k, o) in enumerate(objs.items()):
        h = o["hash"]
        total += fetch_file(f"https://resources.download.minecraft.net/{h[:2]}/{h}", h, f"{a.root}/assets/objects/{h[:2]}/{h}")
    print(f"fetch ok version={a.version} java={d['javaVersion']['majorVersion']} bytes_downloaded={total} secs={time.time()-t0:.1f}")

def cmd_world(a):
    """Generate a flat world with the official server jar (offline, no accounts), copy into client saves."""
    sd = f"{a.root}/server"
    open(f"{sd}/eula.txt", "w").write("eula=true\n")
    open(f"{sd}/server.properties", "w").write(
        "level-type=minecraft\\:flat\nlevel-name=world\nonline-mode=false\nserver-port=25599\n"
        "spawn-protection=0\nmax-players=1\ngenerate-structures=false\nmode=creative\nview-distance=4\n")
    p = subprocess.Popen(["java", "-Xmx1G", "-jar", "server.jar", "nogui"], cwd=sd, stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, text=True)
    t0 = time.time(); done = False
    for line in p.stdout:
        if "Done (" in line: done = True; break
        if time.time() - t0 > 240: break
    if done:
        p.stdin.write("stop\n"); p.stdin.flush()
    try: p.wait(60)
    except subprocess.TimeoutExpired: p.kill()
    if not done: sys.exit("world: server never reported Done")
    dst = f"{a.root}/game/saves/trial"
    shutil.rmtree(dst, ignore_errors=True)
    shutil.copytree(f"{sd}/world", dst)
    print(f"world ok secs={time.time()-t0:.1f}")

def build_cmd(a, d):
    feats = ["has_quick_plays_support", "is_quick_play_singleplayer"]
    cp = [f"{a.root}/libraries/{l['downloads']['artifact']['path']}" for l in d["libraries"] if rule_ok(l.get("rules"))]
    cp.append(f"{a.root}/versions/{a.version}/{a.version}.jar")
    def sub(s):
        m = {"auth_player_name": "Dev", "version_name": a.version, "game_directory": f"{a.root}/game",
             "assets_root": f"{a.root}/assets", "assets_index_name": d["assetIndex"]["id"],
             "auth_uuid": OFFLINE_UUID, "auth_access_token": "0", "clientid": "0", "auth_xuid": "0",
             "user_type": "legacy", "version_type": "release", "resolution_width": "854",
             "resolution_height": "480", "quickPlayPath": f"{a.root}/game/quickplay.json",
             "quickPlaySingleplayer": "trial", "launcher_name": "kf-trial", "launcher_version": "0",
             "natives_directory": f"{a.root}/natives", "classpath": ":".join(cp)}
        return re.sub(r"\$\{(\w+)\}", lambda x: m.get(x.group(1), x.group(0)), s)
    jvm, game = [], []
    for kind, out in (("jvm", jvm), ("game", game)):
        for x in d["arguments"][kind]:
            if isinstance(x, str): out.append(sub(x))
            elif rule_ok(x.get("rules"), feats):
                v = x["value"]; out.extend(sub(i) for i in ([v] if isinstance(v, str) else v))
    return jvm, game, cp

def cmd_run(a):
    d = vjson(a.root, a.version)
    os.makedirs(f"{a.root}/game", exist_ok=True)
    jvm, game, _ = build_cmd(a, d)
    out = jvm
    os.makedirs(f"{a.root}/game/logs", exist_ok=True)
    for f in ("latest.log",): 
        try: os.remove(f"{a.root}/game/logs/{f}")
        except FileNotFoundError: pass
    shutil.rmtree(f"{a.root}/game/crash-reports", ignore_errors=True)
    shutil.rmtree(f"{a.root}/game/screenshots", ignore_errors=True)
    # keep the unbounded vsync/fps default off; small settings
    open(f"{a.root}/game/options.txt", "w").write("onboardAccessibility:false\ntutorialStep:none\npauseOnLostFocus:false\nenableVsync:false\nmaxFps:260\n")
    cmd = ["java", "-Xmx2G"] + out + [d["mainClass"]] + game
    env = dict(os.environ)
    log = open(f"{a.root}/stdout.log", "w")
    t0 = time.time()
    p = subprocess.Popen(cmd, cwd=f"{a.root}/game", env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    # wait until the world is joined (log says it) or timeout
    joined = None; lg = f"{a.root}/game/logs/latest.log"
    while time.time() - t0 < a.join_timeout and p.poll() is None:
        if os.path.exists(lg) and re.search(r"Dev joined the game|Loaded \d+ advancements|Saving is already turned off|\[CHAT\]", open(lg, errors="replace").read()):
            if "joined the game" in open(lg, errors="replace").read(): joined = time.time() - t0; break
        time.sleep(1)
    maps_txt = ""
    alive_after_join = False; shot = f"{a.root}/shot.png"
    if joined:
        time.sleep(a.wait)
        alive_after_join = p.poll() is None
        # renderer evidence: glxinfo on the same display, plus which GL driver libs the JVM mapped
        gi = subprocess.run("glxinfo -B", shell=True, env=env, capture_output=True, text=True).stdout
        m = re.search(r"OpenGL renderer string: (.*)", gi)
        mp = open(f"/proc/{p.pid}/maps").read() if alive_after_join else ""
        libs = sorted(set(re.findall(r"(libnvidia-glcore[^\s]*|libGLX_nvidia[^\s]*|libgallium[^\s]*|swrast[^\s]*|libEGL_nvidia[^\s]*)", mp)))
        maps_txt = f"GLXINFO_RENDERER {m.group(1) if m else 'none'} LIBS {libs}"
        tree = subprocess.run(["xwininfo", "-root", "-tree"], env=env, capture_output=True, text=True).stdout
        open(f"{a.root}/xwininfo.txt", "w").write(tree)
        w = re.search(r"(0x[0-9a-f]+) \"Minecraft[^\"]*\"", tree)
        if w:  # capture just the game window (the Xwayland root cannot be read)
            r = subprocess.run(f"xwd -silent -id {w.group(1)} | magick xwd:- {shot}", shell=True, env=env, capture_output=True, text=True)
            open(f"{a.root}/xwd.err", "w").write(r.stderr)
        if not os.path.exists(shot):
            press_key("F2"); time.sleep(3)  # fallback: in-game screenshot -> game/screenshots/*.png
    if not joined and p.poll() is None:
        os.kill(p.pid, signal.SIGQUIT); time.sleep(3)  # thread dump into stdout.log for diagnosis
    # terminate our own process group only
    if p.poll() is None:
        os.killpg(p.pid, signal.SIGTERM)
        try: p.wait(20)
        except subprocess.TimeoutExpired: os.killpg(p.pid, signal.SIGKILL)
    text = open(lg, errors="replace").read() if os.path.exists(lg) else ""
    text += "\n" + maps_txt
    rend = re.search(r"(?:Backend API|GL_RENDERER|OpenGL).*?:\s*(.*)", text)
    gpu = re.search(r"GLXINFO_RENDERER (.*)", text)
    crash = os.path.isdir(f"{a.root}/game/crash-reports") and os.listdir(f"{a.root}/game/crash-reports")
    gl = (gpu.group(0) if gpu else "none")
    ok_gpu = bool(gpu) and "llvmpipe" not in gl and "NVIDIA GeForce" in gl and "libnvidia-glcore" in gl
    sd_ = f"{a.root}/game/screenshots"
    pics = [os.path.join(sd_, f) for f in os.listdir(sd_)] if os.path.isdir(sd_) else []
    if pics: shutil.copy(max(pics, key=os.path.getmtime), shot)
    shot_ok = os.path.exists(shot) and os.path.getsize(shot) > 20000
    res = dict(joined=joined, alive=alive_after_join, crash=bool(crash), gl=gl.strip()[:160], shot=shot_ok)
    print(json.dumps(res))
    ok = bool(joined) and alive_after_join and not crash and shot_ok and (ok_gpu or a.allow_software)
    print("MC_TRIAL " + ("PASS" if ok else "FAIL") + f" gpu_real={ok_gpu}")
    sys.exit(0 if ok else 1)

ap = argparse.ArgumentParser()
ap.add_argument("cmd", choices=["fetch", "world", "run"])
ap.add_argument("--root", required=True)
ap.add_argument("--version", default="1.21.8")
ap.add_argument("--wait", type=int, default=20)
ap.add_argument("--join-timeout", type=int, default=240)
ap.add_argument("--allow-software", action="store_true", help="do not require a real GPU renderer")
a = ap.parse_args()
{"fetch": cmd_fetch, "world": cmd_world, "run": cmd_run}[a.cmd](a)
