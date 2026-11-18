#!/usr/bin/env python3
"""Drive the Black Myth Wukong Benchmark Tool through its own menu, N attempts, in the running guest (hold session).
Screens are identified by OCR of a guest screenshot (tesseract on the host); every decision and timestamp is logged.
usage: KF_WORK=<name> wukong_drive.py <outdir> <attempts> <qemu.log>"""
import json, os, re, subprocess, sys, time, pathlib, shutil
OUT = pathlib.Path(sys.argv[1]); N = int(sys.argv[2]); QLOG = pathlib.Path(sys.argv[3])
HERE = pathlib.Path(__file__).resolve().parent
OUT.mkdir(parents=True, exist_ok=True)
def gx(*a, timeout=120):
    p = subprocess.run([sys.executable, "-B", "-I", str(HERE / "gx2.py"), *a], capture_output=True, text=True, timeout=timeout, errors="replace")
    return p.stdout
def cmds(lines, wait_done=True, tmo=180):
    f = OUT / "cmd.txt"; f.write_text("\n".join(lines) + "\n")
    gx("ps", r"Remove-Item C:\kfhw\done.txt -ErrorAction SilentlyContinue", "30")
    gx("push", str(f), "C:/kfhw/cmd.txt")
    t0 = time.time()
    while wait_done and time.time() - t0 < tmo:
        if "yes" in gx("ps", r'if(Test-Path C:\kfhw\done.txt){"yes"}', "30"): return True
        time.sleep(1)
    return not wait_done
def pull(remote, local): gx("pull", remote, str(local))
def ocr(png):
    p = subprocess.run(["tesseract", str(png), "-", "--psm", "6"], capture_output=True, text=True, errors="replace")
    return re.sub(r"\s+", " ", p.stdout)
def qlines(): return sum(1 for _ in open(QLOG, errors="replace"))
def status(tag):
    cmds([f"STATUS|{tag}"])
    pull(f"C:/kfhw/status_{tag}.json", OUT / f"status_{tag}.json")
    try: return json.loads((OUT / f"status_{tag}.json").read_text(encoding="utf-8-sig"))
    except Exception: return {}
def screen(tag):
    cmds([f"SHOT|{tag}"]); pull(f"C:/kfhw/{tag}.png", OUT / f"{tag}.png"); return ocr(OUT / f"{tag}.png")
res = []
RW = int(os.environ.get('RELAUNCH_WAIT', '0'))
for att in range(1, N + 1):
    log = open(OUT / f"attempt{att}.log", "w")
    def L(m):
        s = f"{time.strftime('%H:%M:%S')} {m}"; print(s, flush=True); log.write(s + "\n"); log.flush()
    L(f"ATTEMPT {att} qemu.log line {qlines()}")
    base = status("base"); lke0 = len(base.get("lke") or [])
    cmds(["LAUNCH"]); t_launch = time.time(); q_launch = qlines(); tdr_first = None; fps_series = []
    state = "launched"; confirmed_at = None; q_confirm = None; end = None; vram_peak = 0; seen = []
    clicked_bench = 0; clicked_conf = 0; agree_tries = 0
    while time.time() - t_launch < 900:
        time.sleep(3)
        tag = f"a{att}_{int(time.time() - t_launch):04d}"
        st = status("cur")
        with open(OUT / "polls.jsonl", "a") as pj: pj.write(json.dumps(dict(att=att, host=time.time(), guest=st.get("now"), qlines=qlines(), alive=st.get("alive"), nlke=len(st.get("lke") or []), lke=(st.get("lke") or [])[:6])) + "\n")
        m = re.match(r"\s*(\d+)", st.get("smi") or "")
        if m and int(m.group(1)) < 100000: vram_peak = max(vram_peak, int(m.group(1)))
        lke_n = len(st.get("lke") or [])
        if lke_n > lke0 and tdr_first is None:
            tdr_first = time.time(); L(f"LKE count {lke0}->{lke_n}: first GPU watchdog event, qemu.log line {qlines()}")
        if tdr_first and time.time() - tdr_first > 25:
            end = "tdr"; L(f"TDR: {lke_n-lke0} LiveKernelEvents since launch; aborting attempt"); break
        if not st.get("alive") and time.time() - t_launch > 70:
            end = "game_exited"; L(f"game process gone; status={st}"); break
        txt = screen("cur_shot")
        shutil.copy(OUT / "cur_shot.png", OUT / f"{tag}.png")
        key = txt[:160]
        L(f"t={int(time.time()-t_launch)} ocr='{key}'")
        low = txt.lower()
        if "initial setup" in low: cmds(["KEY|e"]); seen.append("lang")
        elif "adjust the brightness" in low: cmds(["KEY|e"]); seen.append("bright")
        elif "data storage and processing" in low:
            agree_tries += 1
            if "thank you for your support" in low or "better gaming experience" in low: cmds(["KEY|e"]); seen.append("agree")
            else: cmds(["WHEEL|60"]); seen.append("scroll")
        elif "press any button" in low: cmds(["KEY|e"]); seen.append("anykey")
        elif "purchase now" in low and "run benchmark" not in low:
            clicked_bench += 1; cmds(["CLICK|150|485"]); seen.append("menu")
        elif "run benchmark" in low:
            clicked_conf += 1; q_confirm = qlines(); confirmed_at = time.time(); cmds(["CLICK|755|635"]); seen.append("confirm"); L(f"CONFIRM clicked, qemu.log line {q_confirm}")
        elif confirmed_at and time.time() - confirmed_at > 60 and "purchase now" in low:
            end = "back_to_menu"; L("menu again after confirm: benchmark finished?"); break
        elif confirmed_at and "current fps" in low:
            seen.append("running"); mm = re.search(r"current fps\s*[^0-9a-z]*(\d+)", low)
            if mm: fps_series.append((int(time.time() - confirmed_at), int(mm.group(1))))
        elif confirmed_at and time.time() - confirmed_at > 120 and any(w in low for w in ("average", "score", "minimum", "result", "grade", "rating")):
            end = "result_screen"; L("result screen text: " + txt[:300]); break
        if confirmed_at and time.time() - confirmed_at > 900: end = "timeout"; break
    if end is None: end = "attempt_timeout"
    q_end = qlines()
    files = gx("ps", r'''Get-ChildItem "C:\Program Files (x86)\Steam\steamapps\common\Black Myth Wukong Benchmark Tool","$env:USERPROFILE\Documents","$env:LOCALAPPDATA" -Recurse -File -ErrorAction SilentlyContinue | Where-Object {$_.LastWriteTime -gt (Get-Date).AddMinutes(-12) -and $_.FullName -notmatch "Shader|Cache|GPUCache|\\Temp\\"} | Select -First 60 | % {$_.FullName+" "+$_.Length+" "+$_.LastWriteTime.ToString("T")}''', "120")
    (OUT / f"files{att}.txt").write_text(files)
    st = status("final")
    L(f"END {end} vram_peak={vram_peak} secs_since_launch={int(time.time()-t_launch)} confirm_to_end={int(time.time()-confirmed_at) if confirmed_at else None} qlog {q_launch}->{q_confirm}->{q_end}")
    # crash context + events
    gx("ps", r'''$c="C:\Program Files (x86)\Steam\steamapps\common\Black Myth Wukong Benchmark Tool\b1\Saved\Crashes"; Get-ChildItem $c -Directory | Sort LastWriteTime | select -Last 3 | % {$_.Name+" "+$_.LastWriteTime.ToString("T")} ''', "60")
    ev = gx("ps", r'''Get-WinEvent -FilterHashtable @{LogName="System";StartTime=(Get-Date).AddMinutes(-20)} -ErrorAction SilentlyContinue | Where-Object {$_.ProviderName -match "Display|nvlddmkm|dxgkrnl|LiveKernel"} | Sort TimeCreated | % {$_.TimeCreated.ToString("T")+" "+$_.ProviderName+" id="+$_.Id}''', "90")
    (OUT / f"events{att}.txt").write_text(ev)
    import struct
    shot_w = struct.unpack('>II', open(OUT/'cur_shot.png','rb').read(24)[16:24])
    res.append(dict(attempt=att, shot_wh=shot_w, fps_series=fps_series, ran=('running' in seen), end=end, seen=seen, vram_peak=vram_peak, confirm_to_end=(int(time.time()-confirmed_at) if confirmed_at else None), qlog=[q_launch, q_confirm, q_end], status=st, events=ev.strip().splitlines()[-8:]))
    (OUT / "summary.json").write_text(json.dumps(res, indent=1))
    log.close()
    if end == "game_exited":
        cmds(["WAIT|20"])   # let the guest settle before the next launch

if RW:
    # pcnight3 E1: immediate relaunch after the completed benchmark; no menu driving, wait RW s, record LKE / alive / marker
    log = open(OUT / "relaunch.log", "w")
    def L(m):
        s = f"{time.strftime('%H:%M:%S')} {m}"; print(s, flush=True); log.write(s + "\n"); log.flush()
    base = status("rbase"); lke0 = len(base.get("lke") or [])
    q0 = qlines(); L(f"RELAUNCH attempt-1 end={res[-1]['end'] if res else None} lke0={lke0} qemu.log line {q0}")
    cmds(["LAUNCH"]); t0 = time.time(); first = None; qfirst = None; alive_last = None
    while time.time() - t0 < RW:
        time.sleep(3)
        st = status("cur")
        n = len(st.get("lke") or []); alive_last = st.get("alive")
        with open(OUT / "polls.jsonl", "a") as pj: pj.write(json.dumps(dict(att=99, host=time.time(), guest=st.get("now"), qlines=qlines(), alive=alive_last, nlke=n, lke=(st.get("lke") or [])[:6], smi=st.get("smi"))) + "\n")
        if n > lke0 and first is None:
            first = time.time() - t0; qfirst = qlines(); L(f"RELAUNCH first LKE at +{first:.1f} s, lke {lke0}->{n}, qemu.log line {qfirst}")
        L(f"t={int(time.time()-t0)} alive={alive_last} lke={n} q={qlines()}")
    L(f"RELAUNCH done: tdr={first is not None} first_lke_s={first} alive_end={alive_last} qlog {q0}->{qlines()}")
    res.append(dict(relaunch=True, wait=RW, tdr=first is not None, first_lke_s=first, alive_end=alive_last, qlog=[q0, qfirst, qlines()]))
    (OUT / "summary.json").write_text(json.dumps(res, indent=1)); log.close()
print(json.dumps(res, indent=1))
