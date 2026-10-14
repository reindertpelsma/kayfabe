#!/usr/bin/env python3
"""cdp_load.py — drive a Chromium-family browser (google-chrome or Electron) through a timed mixed GPU load over CDP.

  cdp_load.py --kind chrome|electron --name N --minutes M --outdir D [--flags "..."] [--no-webgpu] [--skip-fullscreen]

Phases (relative minutes, scaled to --minutes; every event is logged with a wall time to D/events.log):
  0     launch, SystemInfo.getInfo (feature status) -> gpuinfo.json, chrome://gpu text -> gpu_report.txt
  0+    one tab "mix.html": WebGL2 instanced scene + canvas2d + CSS animations/backdrop-filter + scrolling DOM + H.264 video
  20%   video -> VP9, 30% -> AV1 (software or hardware decode as the flags allow)
  40%   fullscreen video (userGesture) for 8% of the run, then exit fullscreen
  50%   tab/window churn: 24 extra tabs/windows (mix of small WebGL, CSS, video), closed again
  70%   kill -9 the GPU process (once), check recovery and that the page renders again
  80%   WebGPU tab (unless --no-webgpu)
  rest  steady mixed load
Sampling every 10 s: page fps counters, video quality, GPU process pid, tab crash events, guest dmesg counters.
"""
import argparse, json, os, re, signal, subprocess, sys, time, urllib.request
import websocket  # python3-websocket

ap = argparse.ArgumentParser()
ap.add_argument('--kind', default='chrome')
ap.add_argument('--name', required=True)
ap.add_argument('--minutes', type=float, default=11)
ap.add_argument('--outdir', required=True)
ap.add_argument('--flags', default='')
ap.add_argument('--no-webgpu', action='store_true')
ap.add_argument('--skip-fullscreen', action='store_true')
ap.add_argument('--port', type=int, default=9222)
ap.add_argument('--hash', default='scr=1', help='extra mix.html hash parameters, e.g. glN=1500')
a = ap.parse_args()
os.makedirs(a.outdir, exist_ok=True)
KF = os.path.expanduser('~/kf')
EV = open(os.path.join(a.outdir, 'events.log'), 'a', buffering=1)
MET = open(os.path.join(a.outdir, 'metrics.jsonl'), 'a', buffering=1)
T0 = time.time()


def ev(msg):
    line = f'{time.strftime("%H:%M:%S")} +{time.time()-T0:6.0f}s {msg}'
    print(line, flush=True)
    EV.write(line + '\n')


def sh(cmd):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True).stdout


def dmesg_counts():
    d = sh('sudo dmesg 2>/dev/null')
    return {
        'xid': len(re.findall(r'Xid', d)),
        'nvrm_lines': len(re.findall(r'NVRM', d)),
        'flip_timeout': len(re.findall(r'Flip event timeout', d)),
        'gsp_timeout': len(re.findall(r'(?i)gsp.*timeout|timeout.*gsp', d)),
    }


def gpu_pids():
    return sorted(int(p) for p in sh('pgrep -f "[t]ype=gpu-process"').split())


# ---------------- launch
env = dict(os.environ, DISPLAY=':0', XAUTHORITY='/home/ubuntu/.Xauthority')
prof = f'/tmp/prof-{a.name}'
sh(f'rm -rf {prof}')
common = f'--remote-debugging-port={a.port} --remote-allow-origins=* --user-data-dir={prof} --no-first-run --no-default-browser-check --password-store=basic --window-size=1600,900 --window-position=0,0 --autoplay-policy=no-user-gesture-required --allow-file-access-from-files'
if a.kind == 'chrome':
    cmd = f'google-chrome-stable {common} {a.flags} about:blank'
else:
    cmd = f'{KF}/electron/electron --no-sandbox {common} {a.flags} {KF}/electron-app'
ev(f'LAUNCH {cmd}')
logf = open(os.path.join(a.outdir, 'browser.log'), 'w')
proc = subprocess.Popen(cmd, shell=True, env=env, stdout=logf, stderr=subprocess.STDOUT, preexec_fn=os.setsid)
base = f'http://127.0.0.1:{a.port}'
for _ in range(60):
    try:
        ver = json.load(urllib.request.urlopen(base + '/json/version', timeout=2))
        break
    except Exception:
        time.sleep(1)
else:
    ev('FAIL browser never exposed CDP'); sys.exit(3)
ev(f'BROWSER {ver.get("Browser")} {ver.get("User-Agent")}')


class CDP:
    def __init__(self, url):
        self.ws = websocket.create_connection(url, timeout=30, suppress_origin=True)
        self.id = 0
        self.events = []

    def call(self, method, params=None, timeout=30):
        self.id += 1
        i = self.id
        self.ws.send(json.dumps({'id': i, 'method': method, 'params': params or {}}))
        self.ws.settimeout(timeout)
        while True:
            m = json.loads(self.ws.recv())
            if m.get('id') == i:
                return m
            if 'method' in m:
                self.events.append(m)


browser = CDP(ver['webSocketDebuggerUrl'])
crashes = []


def drain_events(c):
    c.ws.settimeout(0.05)
    try:
        while True:
            m = json.loads(c.ws.recv())
            c.events.append(m)
    except Exception:
        pass
    ev_ = c.events
    c.events = []
    return ev_


def tab_ws(target_id):
    return CDP(f'ws://127.0.0.1:{a.port}/devtools/page/{target_id}')


def targets():
    return [t for t in json.load(urllib.request.urlopen(base + '/json/list', timeout=5)) if t.get('type') == 'page']


opener = None   # electron: the first window, used to window.open() further ones (no Target.createTarget there)


def new_tab(url, window=False):
    if a.kind == 'electron':
        before = {t['id'] for t in targets()}
        opener.call('Runtime.evaluate', {'expression': f'window.open({json.dumps(url)},"_blank","width=800,height=450")!==null', 'userGesture': True, 'returnByValue': True})
        for _ in range(20):
            new = [t for t in targets() if t['id'] not in before]
            if new:
                return new[0]['id']
            time.sleep(0.5)
        raise RuntimeError('electron: window.open produced no target')
    r = browser.call('Target.createTarget', {'url': url, 'newWindow': window})
    return r['result']['targetId']


def close_tab(tid):
    if a.kind == 'electron':
        c = tab_ws(tid)
        try:
            c.call('Runtime.evaluate', {'expression': 'window.close()', 'userGesture': True})
        finally:
            c.ws.close()
    else:
        browser.call('Target.closeTarget', {'targetId': tid})


DEEP = r"""(()=>{const out=[];function w(n,d){if(!n||d>40)return;if(n.nodeType===3){const t=n.textContent.trim();if(t)out.push(t)}
 if(n.shadowRoot)w(n.shadowRoot,d+1);for(const c of (n.childNodes||[]))w(c,d+1);}w(document.documentElement,0);return out.join('\n')})()"""

# ---------------- feature status + chrome://gpu
try:
    si = browser.call('SystemInfo.getInfo')
    json.dump(si, open(os.path.join(a.outdir, 'gpuinfo.json'), 'w'), indent=1)
    fs = si.get('result', {}).get('gpu', {}).get('featureStatus', {})
    ev('FEATURESTATUS ' + json.dumps(fs))
    aux = si.get('result', {}).get('gpu', {}).get('auxAttributes', {})
    devs = [(d.get('vendorString'), d.get('deviceString'), d.get('driverVersion')) for d in si.get('result', {}).get('gpu', {}).get('devices', [])]
    ev(f'GPUDEVICES {devs} gl_renderer={aux.get("glRenderer")} gl_version={aux.get("glVersion")}')
except Exception as e:
    ev(f'getInfo failed {e}')
if a.kind == 'electron':
    opener = tab_ws(targets()[0]['id'])
try:
  if a.kind == 'electron':
    raise RuntimeError('chrome://gpu is not reachable from an Electron window; the feature status above stands in')
  else:
    gt = new_tab('chrome://gpu')
    time.sleep(6)
    c = tab_ws(gt)
    r = c.call('Runtime.evaluate', {'expression': DEEP, 'returnByValue': True})
    open(os.path.join(a.outdir, 'gpu_report.txt'), 'w').write(r['result']['result'].get('value', ''))
    ev('chrome://gpu captured')
    c.ws.close()
except Exception as e:
    ev(f'gpu report failed {e}')
try:
    close_tab(gt)
except Exception:
    pass

# ---------------- main page
mixurl = f'file://{KF}/pages/mix.html#{a.hash}'
mix_id = None
for t in targets():
    if t['url'] in ('about:blank', '') or t['url'].startswith('file://'):
        mix_id = t['id']
        break
if mix_id is None:
    mix_id = new_tab(mixurl)
mixc = tab_ws(mix_id)
mixc.call('Page.enable')
mixc.call('Runtime.enable')
mixc.call('Media.enable')
seen_dec = set()
mixc.call('Page.navigate', {'url': mixurl})
ev(f'LOAD mix.html#{a.hash}')
time.sleep(5)


def ev_js(c, expr, gesture=False):
    r = c.call('Runtime.evaluate', {'expression': expr, 'returnByValue': True, 'userGesture': gesture, 'awaitPromise': False}, timeout=20)
    return r.get('result', {}).get('result', {}).get('value')


total = a.minutes * 60
sched = [
    (0.20, 'video_vp9'), (0.30, 'video_av1'),
    (0.40, 'fullscreen_on'), (0.48, 'fullscreen_off'),
    (0.50, 'churn'),
    (0.70, 'kill_gpu'),
    (0.80, 'webgpu'),
]
if a.skip_fullscreen:
    sched = [s for s in sched if not s[1].startswith('fullscreen')]
done = set()
extra = []
last_sample = 0
gpu0 = gpu_pids()
ev(f'GPU_PIDS {gpu0}')
base_dm = dmesg_counts()
ev(f'DMESG_BASE {base_dm}')
alive = True
webgpu_id = None


def do(action):
    global mixc, webgpu_id
    ev(f'ACTION {action}')
    if action == 'video_vp9':
        ev_js(mixc, "window.__video('vp9')")
    elif action == 'video_av1':
        ev_js(mixc, "window.__video('av1')")
    elif action == 'fullscreen_on':
        ev_js(mixc, "document.getElementById('v')&&document.getElementById('v').requestFullscreen().then(()=>'fs').catch(e=>'fserr '+e)", gesture=True)
        # also tell the OS-level window to take it (kiosk-like); CDP fullscreen is via the Fullscreen API
    elif action == 'fullscreen_off':
        ev_js(mixc, "document.fullscreenElement&&document.exitFullscreen()", gesture=True)
    elif action == 'churn':
        ids = []
        for i in range(24):
            kind = i % 3
            if kind == 0:
                u = f'file://{KF}/pages/mix.html#c2d=0&css=0&vid=none&glN=300'
            elif kind == 1:
                u = f'file://{KF}/pages/mix.html#gl=0&c2d=0&vid=none&scr=1'
            else:
                u = f'file://{KF}/pages/mix.html#gl=0&c2d=0&css=0&vid=h264'
            ids.append(new_tab(u, window=(i % 4 == 0)))
            time.sleep(3)
        time.sleep(20)
        for i in ids:
            try:
                close_tab(i)
            except Exception as e:
                ev(f'close failed {e}')
            time.sleep(0.5)
    elif action == 'kill_gpu':
        pids = gpu_pids()
        ev(f'KILL gpu-process {pids}')
        for p in pids:
            os.kill(p, signal.SIGKILL)
        time.sleep(8)
        ev(f'GPU_PIDS after kill {gpu_pids()}')
    elif action == 'webgpu':
        webgpu_id = new_tab(f'file://{KF}/pages/webgpu.html', window=True)
        time.sleep(5)


def sample():
    s = {'t': round(time.time() - T0)}
    try:
        s['mix'] = ev_js(mixc, 'window.__stats') or {}
        s['vq'] = ev_js(mixc, 'window.__vq&&window.__vq()')
    except Exception as e:
        s['mix_err'] = str(e)[:100]
    if webgpu_id:
        try:
            w = tab_ws(webgpu_id)
            s['wgpu'] = ev_js(w, 'window.__stats')
            w.ws.close()
        except Exception as e:
            s['wgpu_err'] = str(e)[:100]
    s['gpu_pids'] = gpu_pids()
    s['dmesg'] = dmesg_counts()
    for m in drain_events(browser) + drain_events(mixc):
        if m.get('method') == 'Media.playerPropertiesChanged':
            for pr in m['params'].get('properties', []):
                if pr.get('name') in ('kVideoDecoderName', 'kIsPlatformVideoDecoder', 'kVideoCodec'):
                    key = (pr['name'], pr.get('value'))
                    if key not in seen_dec:
                        seen_dec.add(key)
                        ev(f'MEDIA {pr["name"]}={pr.get("value")}')
        if m.get('method') in ('Inspector.targetCrashed', 'Target.targetCrashed'):
            crashes.append(m)
            ev(f'TARGET_CRASHED {json.dumps(m)[:200]}')
    MET.write(json.dumps(s) + '\n')
    return s


while time.time() - T0 - 0 < total + 20:
    el = time.time() - T0 - 14   # ~14 s of setup before the load starts
    for frac, act in sched:
        if act not in done and el >= frac * total:
            done.add(act)
            try:
                do(act)
            except Exception as e:
                ev(f'ACTION {act} failed: {e}')
    if time.time() - last_sample >= 10:
        last_sample = time.time()
        try:
            s = sample()
            m = s.get('mix', {})
            ev(f'SAMPLE gl={m.get("gl")} c2d={m.get("c2d")} css={m.get("css")} scr={m.get("scr")} vq={s.get("vq")} gpu_pids={s.get("gpu_pids")} err={m.get("err")} dm={s["dmesg"]}')
        except Exception as e:
            ev(f'SAMPLE failed {e}')
            # the page may have been replaced (reload after context restore): reconnect
            try:
                ts = [t for t in targets() if t['url'].startswith(mixurl.split('#')[0]) and t['url'].endswith('#' + a.hash)]
                mixc = tab_ws(ts[0]['id'])
            except Exception as e2:
                ev(f'reconnect failed {e2}')
    if el >= total:
        break
    time.sleep(1)

ev('END of load; final sample')
try:
    sample()
except Exception as e:
    ev(f'final sample failed {e}')
fin = dmesg_counts()
ev(f'DMESG_FINAL {fin} delta_xid={fin["xid"]-base_dm["xid"]} delta_nvrm={fin["nvrm_lines"]-base_dm["nvrm_lines"]}')
ev(f'TARGET_CRASHES {len(crashes)} GPU_PIDS_FINAL {gpu_pids()}')
for p in sh('pgrep -af "[g]pu-recent-crash-count"').splitlines()[:2]:
    ev('CMDLINE ' + p[:400])
try:
    os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
except Exception:
    pass
time.sleep(3)
sh('pkill -f "remote-debugging-port=%d"' % a.port)
ev('DONE')
