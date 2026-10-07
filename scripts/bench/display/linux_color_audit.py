#!/usr/bin/env python3
"""Bounded, serial Linux compositor/color experiment on a prepared private overlay.

Run on the GPU host. --base is a powered-off guest with NVIDIA 580.159.04,
Weston/Sway/seatd/wlsunset, Vulkan tools, and Wayland/DRM development packages.
No host display settings are changed. All screenshots are of the guest console.
"""
import argparse
import datetime
import fcntl
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--qemu', type=Path, required=True)
    p.add_argument('--base', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--key', type=Path, required=True)
    p.add_argument('--revision', required=True)
    p.add_argument('--port', type=int, default=2244)
    p.add_argument('--reject-gamma', action='store_true', help='Guest-only KMS failure control')
    p.add_argument('--sdr-color', action='store_true', help='Enable the real bounded SDR GPU colour path')
    p.add_argument('--require-tmo', action='store_true',
                   help='Request a zero-intensity plane TMO LUT through Sway; missing/skipped TMO fails')
    a = p.parse_args()
    if a.require_tmo and a.reject_gamma:
        p.error('TMO and gamma-failure controls are separate experiments')
    if a.require_tmo and not a.sdr_color:
        p.error('The explicit TMO gate requires the real colour path (--sdr-color)')
    if len(a.revision) != 40 or not all(x in '0123456789abcdef' for x in a.revision):
        p.error('A full product revision is required')
    here = Path(__file__).resolve().parent
    scene_source = (here / '../gfxset/src/wl_scene.c').resolve().read_bytes()
    if a.require_tmo:
        # Identical grayscale fixture in all three phases; this is not a
        # compositor shader implementation of the requested TMO transform.
        for old, new in [
            (b'gl_FragColor = vec4(v * (0.6 + 0.4 * gl_FragCoord.z), 1.0);',
             b'float gray = dot(v, vec3(0.2126, 0.7152, 0.0722)) * (0.6 + 0.4 * gl_FragCoord.z); gl_FragColor = vec4(vec3(gray), 1.0);'),
            (b'glClearColor(0.1f, 0.2f, 0.3f, 1.0f);',
             b'glClearColor(0.25f, 0.25f, 0.25f, 1.0f);'),
        ]:
            if scene_source.count(old) != 1:
                p.error('Scene fixture changed; review the grayscale construction')
            scene_source = scene_source.replace(old, new)
    if subprocess.run(['git', '-C', str(here), 'cat-file', '-e', a.revision+'^{commit}'],
                      capture_output=True).returncode:
        p.error('Product revision must name an existing source commit')
    if a.qemu.parent.name != a.revision[:8]:
        p.error('QEMU must be the immutable binary for the named revision')
    if subprocess.run(['pgrep', '-x', 'qemu-system-x86'], capture_output=True).returncode == 0:
        p.error('Another VM is running; hardware work is serial')
    lock = open('/tmp/kayfabe-fastguest.lock', 'w')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    a.out.mkdir(mode=0o700, parents=True, exist_ok=False)
    ssh = ['ssh', '-i', str(a.key), '-p', str(a.port), '-o', 'BatchMode=yes',
           '-o', 'StrictHostKeyChecking=no', '-o', 'UserKnownHostsFile=/dev/null',
           '-o', 'LogLevel=ERROR', '-o', 'ConnectTimeout=5', 'ubuntu@127.0.0.1']
    events = []

    def mark(label):
        event = dict(label=label, utc=datetime.datetime.now(datetime.timezone.utc).isoformat())
        events.append(event)
        (a.out / 'events.json').write_text(json.dumps(events, indent=2) + '\n')
        print('COLOR_EVENT', event, flush=True)

    def guest(label, command, timeout=60, check=True, data=None):
        mark(label)
        r = subprocess.run(ssh + [command], input=data, capture_output=True, timeout=timeout)
        (a.out / (label + '.log')).write_bytes(r.stdout + r.stderr)
        if check and r.returncode:
            raise RuntimeError(f'{label}: guest exit {r.returncode}; see log')
        return r

    def shot(label):
        mark(label)
        with socket.socket(socket.AF_UNIX) as s:
            s.settimeout(10)
            s.connect(str(a.out / 'qmp.sock'))
            f = s.makefile('rwb')
            f.readline()
            for command in [dict(execute='qmp_capabilities'),
                            dict(execute='screendump', arguments=dict(
                                filename=str(a.out / (label + '.ppm')), device='kf0'))]:
                f.write((json.dumps(command) + '\n').encode()); f.flush()
                while True:
                    reply = json.loads(f.readline())
                    if 'error' in reply: raise RuntimeError(reply)
                    if 'return' in reply: break
        if a.require_tmo:
            with (a.out / 'qemu.log').open('rb') as trace:
                data = trace.read(128 * 1024 * 1024 + 1)
            if len(data) > 128 * 1024 * 1024:
                raise ValueError('Trace exceeds experiment bound')
            (a.out / (label + '-trace.log')).write_bytes(data)

    subprocess.run(['qemu-img', 'create', '-f', 'qcow2', '-F', 'qcow2', '-b',
                    str(a.base), str(a.out / 'guest.qcow2')], check=True)
    cmd = [str(a.qemu), '-machine', 'q35,accel=kvm', '-cpu', 'host',
           '-bios', '/usr/share/seabios/bios-256k.bin', '-m', '6G', '-smp', '6',
           '-object', 'memory-backend-memfd,id=ram,size=6G,share=on',
           '-machine', 'memory-backend=ram', '-vga', 'none', '-device',
           'kf3-gpu,guest-driver=580.159.04,fb-mb=4096,bar1-size=134217728,'
           'bar2-size=33554432,id=kf0,display=on', '-display', 'none',
           '-drive', f'if=virtio,file={a.out}/guest.qcow2,format=qcow2',
           '-netdev', f'user,id=n0,hostfwd=tcp:127.0.0.1:{a.port}-:22',
           '-device', 'virtio-net-pci,netdev=n0,romfile=', '-serial',
           f'file:{a.out}/serial.log', '-qmp', f'unix:{a.out}/qmp.sock,server=on,wait=off',
           '-msg', 'timestamp=on']
    env = {k: v for k, v in os.environ.items() if not k.startswith('KF3_')}
    env['KF3_DISPLAY_METHOD_TRACE'] = '1'
    if a.sdr_color:
        env['KF3_DISPLAY_SDR_COLOR'] = '1'
    sha = hashlib.sha256(a.qemu.read_bytes()).hexdigest()
    (a.out / 'manifest.json').write_text(json.dumps(dict(
        source_revision=a.revision, qemu_sha256=sha, command=cmd,
        harness_revision=subprocess.check_output(['git', '-C', str(here), 'rev-parse', 'HEAD'], text=True).strip(),
        harness_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        guest_gamma_fault=a.reject_gamma,
        required_tmo=a.require_tmo, scene_grayscale=a.require_tmo,
        scene_sha256=hashlib.sha256(scene_source).hexdigest(),
        flags={k:v for k,v in env.items() if k.startswith('KF3_')}, scope='AD104 / Linux580.159.04'), indent=2)+'\n')
    mark('start')
    with open(a.out / 'qemu.log', 'wb') as log:
        vm = subprocess.Popen(cmd, env=env, stdout=log, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic()+120
            while time.monotonic() < deadline:
                if vm.poll() is not None: raise RuntimeError('VM exited before SSH')
                if subprocess.run(ssh+['true'], capture_output=True, timeout=8).returncode == 0: break
                time.sleep(2)
            else: raise TimeoutError('Guest SSH')
            guest('driver', 'sudo rmmod nvidia_drm nvidia_modeset 2>/dev/null || true; '
                  'sudo modprobe nvidia-drm modeset=1 fbdev=1; '
                  'sudo cat /sys/module/nvidia_drm/parameters/modeset; '
                  'nvidia-smi --query-gpu=name,driver_version --format=csv,noheader; '
                  'modetest -M nvidia-drm -c -p', timeout=120)
            sources = ['color_properties.c', 'reject_gamma.c', '../gfxset/src/wl_scene.c']
            if a.require_tmo:
                sources.append('request_tmo.c')
            for relative in sources:
                path = (here / relative).resolve()
                guest('copy-'+path.stem, 'mkdir -p ~/color; cat > ~/color/'+path.name,
                      data=scene_source if path.name == 'wl_scene.c' else path.read_bytes())
            guest('build-clients', 'cd ~/color; '
                  'wayland-scanner client-header /usr/share/wayland-protocols/stable/xdg-shell/xdg-shell.xml xdg-shell-client-protocol.h && '
                  'wayland-scanner private-code /usr/share/wayland-protocols/stable/xdg-shell/xdg-shell.xml xdg-shell-protocol.c && '
                  'gcc -O2 -Wall -Wextra -Werror -o color_properties color_properties.c $(pkg-config --cflags --libs libdrm) && '
                  'gcc -O2 -Wall -Wextra -Werror -shared -fPIC -o reject_gamma.so reject_gamma.c $(pkg-config --cflags --libs libdrm) -ldl && '
                  'gcc -O2 -Wall -o wl_scene wl_scene.c xdg-shell-protocol.c -lwayland-client -lwayland-egl -lEGL -lGLESv2')
            if a.require_tmo:
                guest('build-tmo', 'cd ~/color && gcc -O2 -Wall -Wextra -Werror -shared -fPIC '
                      '-o request_tmo.so request_tmo.c $(pkg-config --cflags --libs libdrm) -ldl')
            guest('kms-before', '~/color/color_properties /dev/dri/card0')
            guest('weston-start', "sudo mkdir -m 755 /run/kfcolorw; "
                  "sudo sh -c 'XDG_RUNTIME_DIR=/run/kfcolorw LIBSEAT_BACKEND=builtin nohup weston "
                  "--backend=drm --continue-without-input --socket=kfcolor --log=/tmp/kfcolor-weston.log "
                  ">/tmp/kfcolor-weston.out 2>&1 &'")
            time.sleep(5)
            guest('weston-scene', "sudo sh -c 'XDG_RUNTIME_DIR=/run/kfcolorw WAYLAND_DISPLAY=kfcolor "
                  "nohup /home/ubuntu/color/wl_scene 60 >/tmp/kfcolor-weston-scene.log 2>&1 &'")
            time.sleep(5)
            shot('weston-scene')
            result = guest('weston-result', 'cat /tmp/kfcolor-weston-scene.log; '
                  'sudo cat /tmp/kfcolor-weston.log; ~/color/color_properties /dev/dri/card0')
            if b'WL_SCENE_READY' not in result.stdout or b'GL renderer: NVIDIA' not in result.stdout:
                raise RuntimeError('Weston GPU scene did not become ready')
            guest('weston-stop', 'sudo pkill -x wl_scene || true; sudo pkill -x weston || true')
            time.sleep(3)
            preload = 'LD_PRELOAD=/home/ubuntu/color/reject_gamma.so ' if a.reject_gamma else ''
            if a.require_tmo:
                guest('tmo-control-off', "printf 'off\\n' > ~/color/tmo-mode")
                preload = 'LD_PRELOAD=/home/ubuntu/color/request_tmo.so '
            background = '#808080' if a.require_tmo else '#203040'
            guest('sway-start', "mkdir -p ~/color/runtime; chmod 700 ~/color/runtime; "
                  "printf 'output * bg " + background + " solid_color\\ndefault_border none\\n"
                  "seat seat0 hide_cursor 100\\nxwayland disable\\n' > ~/color/sway.conf; "
                  "sudo systemctl stop seatd; "
                  "sudo sh -c 'SEATD_VTBOUND=0 nohup seatd -g video "
                  ">/tmp/kfcolor-seatd.log 2>&1 &' ; sleep 1; "
                  "XDG_RUNTIME_DIR=/home/ubuntu/color/runtime LIBSEAT_BACKEND=seatd SEATD_SOCK=/run/seatd.sock "
                  + preload + "nohup sway --unsupported-gpu -d -D noscanout -c ~/color/sway.conf >/tmp/kfcolor-sway.log 2>&1 &")
            time.sleep(6)
            wl = 'XDG_RUNTIME_DIR=/home/ubuntu/color/runtime WAYLAND_DISPLAY=wayland-1 '
            guest('sway-scene', wl+'nohup ~/color/wl_scene 180 >/tmp/kfcolor-sway-scene.log 2>&1 &')
            time.sleep(5)
            shot('sway-before')
            result = guest('sway-before', 'cat /tmp/kfcolor-sway-scene.log; '
                  '~/color/color_properties /dev/dri/card0 --dump-lut; '
                  + ('cat /tmp/kfcolor-sway.log; ' if a.require_tmo else 'tail -n 120 /tmp/kfcolor-sway.log; ')
                  +
                  'sudo cat /tmp/kfcolor-seatd.log; pgrep -x sway')
            if b'WL_SCENE_READY' not in result.stdout:
                raise RuntimeError('Sway scene did not become ready; gamma experiment is invalid')
            if a.require_tmo:
                guest('warm-start', "printf 'on\\n' > ~/color/tmo-mode; pkill -x wl_scene || true; sleep 1; "
                      + wl + 'nohup ~/color/wl_scene 180 >/tmp/kfcolor-tmo-scene.log 2>&1 &')
            else:
                guest('warm-start', wl+'WAYLAND_DEBUG=1 nohup wlsunset -t 2500 -T 2501 -l 0 -L 0 '
                      '>/tmp/kfcolor-wlsunset.log 2>&1 &')
            time.sleep(5)
            shot('sway-warm')
            result = guest('sway-warm', '~/color/color_properties /dev/dri/card0 --dump-lut; '
                  + ('cat /tmp/kfcolor-tmo-scene.log; ' if a.require_tmo else 'cat /tmp/kfcolor-wlsunset.log; ')
                  + 'tail -n 200 /tmp/kfcolor-sway.log')
            if a.reject_gamma and b'COLOR_FAULT reject GAMMA_LUT' not in result.stdout:
                raise RuntimeError('KMS failure control was not exercised; result is invalid')
            if a.require_tmo:
                guest('warm-stop', "printf 'restore\\n' > ~/color/tmo-mode; pkill -x wl_scene || true; sleep 1; "
                      + wl + 'nohup ~/color/wl_scene 180 >/tmp/kfcolor-restored-scene.log 2>&1 &')
            else:
                guest('warm-stop', 'pkill -x wlsunset || true')
            time.sleep(3)
            shot('sway-restored')
            guest('sway-restored', '~/color/color_properties /dev/dri/card0 --dump-lut; '
                  + ('cat /tmp/kfcolor-restored-scene.log; ' if a.require_tmo else 'cat /tmp/kfcolor-sway-scene.log; ')
                  + 'cat /tmp/kfcolor-sway.log; '
                  'pgrep -x sway; pgrep -x wl_scene', check=not a.require_tmo)
        finally:
            try:
                guest('dmesg-after', 'sudo dmesg', check=False)
                guest('stop', 'pkill -x wlsunset || true; pkill -x wl_scene || true; '
                      'pkill -x sway || true; sudo pkill -x weston || true; sudo poweroff', check=False)
                vm.wait(timeout=45)
            except (subprocess.TimeoutExpired, OSError):
                vm.terminate()
                try: vm.wait(timeout=15)
                except subprocess.TimeoutExpired: vm.kill(); vm.wait()
            mark('exit-'+str(vm.returncode))
    if a.require_tmo:
        table = here.parents[2] / 'crates/kf-disp/data/classes-580.159.04.tsv'
        verdict = subprocess.run(['python3', str(here / 'check_tmo_stage.py'), str(a.out),
                                  str(table), str(a.out / 'tmo-verdict.json')])
        if verdict.returncode:
            raise SystemExit(verdict.returncode)


if __name__ == '__main__':
    main()
