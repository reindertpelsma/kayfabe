#!/usr/bin/env python3
"""BETA CANDIDATE launcher/preflight and private prefix installation; no env boot flags."""
import argparse
import csv
import fcntl
import grp
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import shlex
import shutil
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
VERSION = (ROOT / 'VERSION').read_text().strip()
MIB = 1 << 20


def parser():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--version', action='version', version='kayfabe '+VERSION)
    p.add_argument('--disk', required=True)
    p.add_argument('--guest-driver', required=True, help='guest ABI version; Windows 580.88 uses 580.65.06')
    p.add_argument('--os', choices=['windows', 'linux'], default='linux')
    p.add_argument('--ram', default='8G')
    p.add_argument('--cpus', type=int, default=8)
    display = p.add_mutually_exclusive_group()
    display.add_argument('--broker', help='broker socket; Windows defaults to the runtime-dir socket')
    display.add_argument('--no-display', action='store_true')
    p.add_argument('--fb-mb', type=int)
    p.add_argument('--bar1-size', type=int)
    p.add_argument('--qemu', type=Path)
    p.add_argument('--prefix', type=Path, default=ROOT)
    p.add_argument('--gpu-minor', type=int, default=0)
    p.add_argument('--network', choices=['on','off'],default='off')
    p.add_argument('--uefi-code', type=Path)
    p.add_argument('--uefi-vars-template', type=Path)
    p.add_argument('--iso', type=Path)
    p.add_argument('--virtio-iso', type=Path)
    p.add_argument('--print', action='store_true', dest='print_only')
    p.add_argument('--facts', type=Path, help='offline facts fixture; permitted only with --print')
    return p


def geometry(facts, display):
    """Documented single-device budget; actual RM allocation remains authoritative."""
    fb = min((int(facts['total_mb']) - 2048) // 1024 * 1024, 8192)
    # BAR2 + PRAMIN + measured context headroom; GOP additionally overlaps one boot frame.
    remaining = int(facts['bar1_mb']) * MIB - (32 + 1 + 16) * MIB
    if display:
        remaining -= 1920 * 1080 * 4
    bar1 = 1 << (remaining.bit_length() - 1) if remaining > 0 else 0
    if fb <= 0 or bar1 < 128 * MIB:
        raise ValueError('insufficient VRAM/BAR1 for the documented device budget')
    return fb, bar1, 32 * MIB


def state_dir(options, facts=None):
    disk = Path(options.disk).resolve()
    ident = uuid.uuid5(uuid.NAMESPACE_URL, (facts or {}).get('machine_id','fixture')+':'+str(disk))
    return disk.parent / '.kayfabe' / str(ident), ident


def qemu_command(options, facts, prefix):
    if not re.fullmatch(r'[1-9][0-9]*[MG]', options.ram) or not 1 <= options.cpus <= 256:
        raise ValueError('RAM must be a positive number with M/G suffix; CPUs must be 1..256')
    if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+', options.guest_driver):
        raise ValueError('guest-driver needs the measured Linux ABI version, such as 580.65.06')
    if options.gpu_minor < 0:
        raise ValueError('GPU minor must be nonnegative')
    display = not options.no_display and (options.os == 'windows' or bool(options.broker))
    fb, bar1, bar2 = geometry(facts, display)
    fb = options.fb_mb if options.fb_mb is not None else fb
    bar1 = options.bar1_size if options.bar1_size is not None else bar1
    if fb <= 0 or bar1 < 128*MIB or bar1 & (bar1-1):
        raise ValueError('explicit framebuffer must be positive and BAR1 a power of two >=128 MiB')
    escape = lambda value: str(value).replace(',', ',,')
    state, ident = state_dir(options,facts)
    qemu = options.qemu or prefix / 'bin/qemu-system-x86_64'
    device = (f'kf3-gpu,id=gpu0,guest-os={options.os},guest-driver={options.guest_driver},'
              f'gpu-minor={options.gpu_minor},fb-mb={fb},bar1-size={bar1},bar2-size={bar2}')
    if display:
        socket = options.broker or str(Path(facts['runtime'])/'kayfabe/broker.sock')
        device += f',display=on,gop=on,display-broker={escape(socket)},display-broker-uid={facts["uid"]}'
    machine = 'pc' if options.os == 'windows' else 'q35'
    command = [str(qemu), '-machine', f'{machine},accel=kvm,memory-backend=ram0',
               '-object', f'memory-backend-memfd,id=ram0,size={options.ram},share=on',
               '-m', options.ram, '-smp', str(options.cpus), '-cpu', 'host,-vmx',
               '-uuid', str(ident), '-smbios', f'type=1,uuid={ident},serial=kayfabe-{ident.hex}', '-nodefaults', '-device', device,
               '-display', 'none', '-vga', 'none',
               '-object', 'iothread,id=diskio',
               '-drive', f'file={escape(options.disk)},if=none,id=disk0,format=qcow2,cache=none',
               '-device', 'virtio-blk-pci,drive=disk0,iothread=diskio',
               '-netdev', 'user,id=net0,restrict='+('off' if options.network=='on' else 'on'),
               '-device', 'virtio-net-pci,netdev=net0,mac=02:'+':'.join(f'{byte:02x}' for byte in ident.bytes[:5]),
               '-device', 'qemu-xhci,id=usb0', '-device', 'usb-tablet,bus=usb0.0',
               '-device', 'usb-kbd,bus=usb0.0',
               '-qmp', f'unix:{state}/qmp.sock,server=on,wait=off',
               '-serial', f'file:{state}/serial.log', '-no-reboot']
    if display or options.os == 'windows' or options.uefi_code:
        code = options.uefi_code or facts.get('uefi_code')
        if not code:
            raise ValueError('UEFI firmware not found; set --uefi-code and --uefi-vars-template')
        command += ['-drive', f'if=pflash,format=raw,readonly=on,file={escape(code)}',
                    '-drive', f'if=pflash,format=raw,file={escape(state/"OVMF_VARS.fd")}']
    if options.os == 'windows':
        command += ['-chardev', f'socket,id=chrtpm,path={state}/swtpm.sock',
                    '-tpmdev', 'emulator,id=tpm0,chardev=chrtpm', '-device', 'tpm-crb']
    for image in [options.iso, options.virtio_iso]:
        if image:
            command += ['-drive', f'file={escape(image)},media=cdrom,readonly=on']
    return command


def doctor_rows(facts, version, display):
    rows = []
    def row(check, status, detail, fix='none'):
        rows.append(dict(check=check, status=status, detail=str(detail), fix=fix))
    row('kvm', 'PASS' if facts.get('kvm') else 'FAIL', '/dev/kvm read/write',
        'sudo usermod -aG kvm "$USER"; log out and back in')
    row('permissions', 'PASS' if facts.get('devices') else 'FAIL', 'NVIDIA control/GPU/UVM device access',
        'ls -l /dev/nvidia* /dev/kvm; sudo usermod -aG video,render,kvm "$USER"')
    row('udmabuf', 'PASS' if not display or facts.get('udmabuf') else 'WARN',
        '/dev/udmabuf optional DMA-BUF tier; shared memory fallback remains available',
        'ls -l /dev/udmabuf; sudo usermod -aG kvm "$USER"; log out and back in')
    driver = facts.get('driver', 'unknown')
    matrix_path=ROOT/'docs/SUPPORT_MATRIX.tsv'
    matrix=list(csv.DictReader(matrix_path.read_text().splitlines(),delimiter='\t')) if matrix_path.exists() else []
    matched=[entry for entry in matrix if entry['host_driver']==driver and entry['family']==facts.get('family')]
    verdict='STALE evidence row' if matched else 'UNTESTED matrix cell'
    row('driver', 'WARN', f'{driver}: {verdict}; no C6 hardware acceptance',
        'cat docs/SUPPORT_MATRIX.md')
    row('gpu', 'PASS' if facts.get('family') in ['Turing','Ampere','Ada','Blackwell'] else 'WARN',
        f'{facts.get("model", "unknown")} / {facts.get("family", "unknown")}', 'nvidia-smi -L')
    required_fb=facts.get('requested_fb', min((facts.get('total_mb',0)-2048)//1024*1024,8192))
    row('vram', 'PASS' if required_fb>0 and facts.get('free_mb', 0)>=required_fb else 'FAIL',
        f'{facts.get("free_mb", 0)} MiB free; store requests {required_fb} MiB; RM remains authoritative', 'nvidia-smi')
    match = facts.get('qemu_version') == version and facts.get('broker_version') == version
    row('versions', 'PASS' if match else 'FAIL',
        f'expected {version}; QEMU={facts.get("qemu_version")} broker={facts.get("broker_version")}',
        'reinstall QEMU, broker and launcher from one release directory')
    row('session', 'PASS' if not display or facts.get('graphical') else 'FAIL',
        'broker must run in the desktop user graphical session', 'loginctl session-status')
    row('disk', 'PASS' if facts.get('disk_free', 0) >= 1<<30 else 'FAIL',
        f'{facts.get("disk_free", 0)} bytes free; VM image/build space is additional', 'df -h .')
    row('groups', 'PASS', ','.join(facts.get('groups', [])), 'id')
    row('hugepages', 'PASS', f'{facts.get("hugepages", 0)} configured; not required by measured rootless arm')
    row('memlock', 'WARN', f'{facts.get("memlock", 0)} bytes; no raised limit required in older rootless arm', 'ulimit -l')
    row('memcg', 'WARN' if facts.get('memcg') in ['max', None] else 'PASS',
        f'memory.max={facts.get("memcg")}; launcher does not invent an unmeasured QEMU overhead allowance',
        'systemctl --user show -p ControlGroup')
    return rows


def collect_facts(prefix, minor=0):
    """Read-only probes: no context creation, VM, driver installation or permission changes."""
    def run(command):
        try:
            result = subprocess.run(command, capture_output=True, text=True, timeout=10)
            return result.stdout.strip() if result.returncode == 0 else ''
        except (OSError, subprocess.TimeoutExpired):
            return ''
    query = run(['nvidia-smi', '-i', str(minor), '--query-gpu=name,driver_version,memory.total,memory.free,pci.bus_id',
                 '--format=csv,noheader,nounits'])
    fields = next(csv.reader(query.splitlines()), [])
    if len(fields) != 5:
        raise ValueError('nvidia-smi did not return one GPU row; check the driver and --gpu-minor')
    model, driver, total, free, bdf = [field.strip() for field in fields]
    pci = Path('/sys/bus/pci/devices') / bdf.lower().replace('00000000:', '0000:')
    resources = (pci/'resource').read_text().splitlines()
    lo, hi, _ = [int(part,16) for part in resources[1].split()]
    family = next((f for token,f in [('RTX 20','Turing'),('RTX 30','Ampere'),('RTX 40','Ada'),('RTX 50','Blackwell')]
                   if token in model), 'unknown (not inferred from a neighbor)')
    versions = {}
    for key, command in [('qemu_version',[str(prefix/'bin/qemu-system-x86_64'),'-device','help']),
                         ('broker_version',[str(prefix/'bin/nvkvm-display-broker'),'--version'])]:
        output = run(command)
        versions[key] = VERSION if f'kayfabe {VERSION}' in output else 'missing or mismatched'
    firmware = next((base for base in [Path('/usr/share/OVMF'),Path('/usr/share/edk2/ovmf')]
                     if (base/'OVMF_CODE_4M.fd').is_file()), None)
    meminfo = dict(line.split(':',1) for line in Path('/proc/meminfo').read_text().splitlines())
    memcg = None
    for line in Path('/proc/self/cgroup').read_text().splitlines():
        if line.startswith('0::'):
            path = Path('/sys/fs/cgroup') / line[3:].lstrip('/') / 'memory.max'
            if path.exists():memcg=path.read_text().strip()
    facts = dict(total_mb=int(float(total)),free_mb=int(float(free)),bar1_mb=(hi-lo+1)//MIB,
                 driver=driver,model=model,family=family,kvm=os.access('/dev/kvm',os.R_OK|os.W_OK),
                 devices=all(os.access(path,os.R_OK|os.W_OK) for path in ['/dev/nvidiactl',f'/dev/nvidia{minor}','/dev/nvidia-uvm']),
                 udmabuf=os.access('/dev/udmabuf',os.R_OK|os.W_OK),
                 graphical=bool(os.environ.get('WAYLAND_DISPLAY') or os.environ.get('DISPLAY')),
                 disk_free=shutil.disk_usage(Path.cwd()).free,hugepages=int(meminfo.get('HugePages_Total','0').strip()),
                 memlock=resource.getrlimit(resource.RLIMIT_MEMLOCK)[0],memcg=memcg,
                 groups=[grp.getgrgid(gid).gr_name for gid in os.getgroups()],
                 machine_id=Path('/etc/machine-id').read_text().strip(),
                 runtime=os.environ.get('XDG_RUNTIME_DIR',f'/run/user/{os.getuid()}'),uid=os.getuid(),**versions)
    if firmware:
        facts.update(uefi_code=str(firmware/'OVMF_CODE_4M.fd'),uefi_vars_template=str(firmware/'OVMF_VARS_4M.fd'))
    return facts


def install_prefix(source, prefix):
    source, prefix = Path(source).resolve(), Path(prefix).absolute()
    if prefix in [Path('/'),Path('/usr'),Path('/usr/bin'),Path('/opt')] or prefix.exists():
        raise ValueError('use a new private versioned prefix; existing destinations are never replaced')
    if not (source/'VERSION').is_file() or not (source/'bin/qemu-system-x86_64').is_file():
        raise ValueError('release directory needs VERSION and bin/qemu-system-x86_64')
    for path in source.rglob('*'):
        if path.is_symlink() and not path.resolve().is_relative_to(source):
            raise ValueError(f'release symlink escapes its directory: {path.relative_to(source)}')
    shutil.copytree(source,prefix,symlinks=False)
    files = {str(path.relative_to(prefix)):hashlib.sha256(path.read_bytes()).hexdigest()
             for path in prefix.rglob('*') if path.is_file()}
    (prefix/'.kayfabe-install.json').write_text(json.dumps(files,indent=2)+'\n')


def uninstall_prefix(prefix):
    prefix=Path(prefix).resolve()
    manifest=prefix/'.kayfabe-install.json'
    files=json.loads(manifest.read_text())
    for name,digest in files.items():
        path=prefix/name
        if Path(name).is_absolute() or '..' in Path(name).parts or not path.resolve().is_relative_to(prefix):
            raise ValueError('unsafe manifest path; nothing outside the prefix is removed')
        if path.is_file() and hashlib.sha256(path.read_bytes()).hexdigest()==digest:
            path.unlink()
    manifest.unlink()
    for path in sorted((p for p in prefix.rglob('*') if p.is_dir()),key=lambda p:len(p.parts),reverse=True):
        try:path.rmdir()
        except OSError:pass
    try:prefix.rmdir()
    except OSError:pass


def main(argv=None):
    argv=list(sys.argv[1:] if argv is None else argv)
    if '--version' in argv:
        print('kayfabe '+VERSION);return 0
    if argv and argv[0] in ['install','uninstall']:
        p=argparse.ArgumentParser();p.add_argument('--prefix',type=Path,required=True);p.add_argument('--from',dest='source',type=Path)
        action=argv.pop(0);options=p.parse_args(argv)
        if action=='install':
            if not options.source:p.error('install needs --from RELEASE_DIRECTORY')
            install_prefix(options.source,options.prefix)
        else:uninstall_prefix(options.prefix)
        return 0
    if argv and argv[0]=='doctor':
        p=argparse.ArgumentParser();p.add_argument('--prefix',type=Path,default=ROOT);p.add_argument('--facts',type=Path)
        p.add_argument('--no-display',action='store_true');p.add_argument('--gpu-minor',type=int,default=0)
        options=p.parse_args(argv[1:]);facts=json.loads(options.facts.read_text()) if options.facts else collect_facts(options.prefix,options.gpu_minor)
        rows=doctor_rows(facts,VERSION,not options.no_display)
        for row in rows:print(f'{row["status"]} {row["check"]}: {row["detail"]}; fix: {row["fix"]}')
        return int(any(row['status']=='FAIL' for row in rows))
    options=parser().parse_args(argv)
    if options.facts and not options.print_only:raise ValueError('--facts is permitted only with --print')
    facts=json.loads(options.facts.read_text()) if options.facts else collect_facts(options.prefix,options.gpu_minor)
    command=qemu_command(options,facts,options.prefix)
    if options.fb_mb is not None:facts['requested_fb']=options.fb_mb
    print(shlex.join(command),flush=True)
    if options.print_only:return 0
    rows=doctor_rows(facts,VERSION,not options.no_display and (options.os=='windows' or bool(options.broker)))
    for row in rows:print(f'{row["status"]} {row["check"]}: {row["detail"]}; fix: {row["fix"]}',file=sys.stderr)
    if any(row['status']=='FAIL' for row in rows):return 1
    if not Path(options.disk).is_file():raise ValueError('disk image does not exist')
    state,_=state_dir(options,facts);state.mkdir(parents=True,exist_ok=True,mode=0o700)
    if state.is_symlink() or state.stat().st_uid!=os.getuid() or state.stat().st_mode & 0o077:
        raise ValueError('VM state directory must be private, owned by this user and not a symlink')
    vm_lock=(state/'vm.lock').open('a')
    fcntl.flock(vm_lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
    template=options.uefi_vars_template or facts.get('uefi_vars_template')
    if any('if=pflash' in part for part in command) and not (state/'OVMF_VARS.fd').exists():
        if not template:raise ValueError('UEFI variable template missing; use --uefi-vars-template')
        shutil.copyfile(template,state/'OVMF_VARS.fd')
    tpm=None
    try:
        if options.os=='windows':
            socket=state/'swtpm.sock'
            if socket.exists():
                if not socket.is_socket():raise ValueError('TPM socket path is not a socket')
                socket.unlink()
            tpm=subprocess.Popen(['swtpm','socket','--tpm2','--tpmstate',f'dir={state}',
                                  '--ctrl',f'type=unixio,path={state}/swtpm.sock'])
            for _ in range(50):
                if (state/'swtpm.sock').exists():break
                if tpm.poll() is not None:raise ValueError('swtpm exited before creating its socket')
                time.sleep(0.1)
            else:raise ValueError('swtpm did not create its socket')
        return subprocess.run(command).returncode
    finally:
        if tpm is not None and tpm.poll() is None:
            tpm.terminate()
            try:tpm.wait(timeout=5)
            except subprocess.TimeoutExpired:tpm.kill();tpm.wait()
        vm_lock.close()


if __name__=='__main__':
    try:sys.exit(main())
    except (ValueError,OSError) as error:
        print(f'FAIL {error}',file=sys.stderr);sys.exit(2)
