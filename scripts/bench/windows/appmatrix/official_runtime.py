#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Pinned box-host official downloads and per-run, automatically removed media.

This module has no upload operation. Only the BOX HOST downloads; guest installer
argv is data for guest staging and is never executed here. J4 output is captured.
"""
import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import re
import urllib.parse
from pathlib import PureWindowsPath

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
import package_bundle as B

class MediaDrift(ValueError):pass

def validate(data):
    seen=set()
    for e in data['entries']:
        if e['id'] in seen:raise MediaDrift('MEDIA_DRIFT duplicate official id')
        seen.add(e['id'])
        if not re.fullmatch(r'[a-z0-9_]+',e['package']):raise MediaDrift('MEDIA_DRIFT invalid package id')
        try:B.validate_pin(e['package'],dict(name=e['name'],url=e['official_url'],sha256=e['sha256'],size=e['size']))
        except ValueError:raise MediaDrift('MEDIA_DRIFT invalid official pin: '+e['package'])from None
        url=urllib.parse.urlsplit(e['official_url']);host=(url.hostname or '').lower()
        allowed=host=='nvidia.com'or host.endswith('.nvidia.com')or host in ('download.pytorch.org','files.pythonhosted.org','aka.ms','download.visualstudio.microsoft.com','sdk.lunarg.com')
        if host=='github.com':allowed=any(url.path.startswith(p)for p in ('/ggml-org/llama.cpp/releases/download/','/krrishnarraj/clpeak/releases/download/'))
        if not allowed:raise MediaDrift('MEDIA_DRIFT non-official origin: '+e['package'])
        destination=PureWindowsPath(e['guest_destination'])
        if '..'in destination.parts or not any(destination.is_relative_to(PureWindowsPath(p))for p in ('C:/kfapps','C:/kf/official')):
            raise MediaDrift('MEDIA_DRIFT guest destination outside application roots')
        if e.get('class')!='official-fetch' or not e['license_id'] or not isinstance(e['installer_argv'],list):
            raise MediaDrift('MEDIA_DRIFT incomplete official pin')
        if not all(isinstance(a,str)and '\x00'not in a for a in e['installer_argv']):raise MediaDrift('MEDIA_DRIFT invalid guest installer argv')
    groups=data.get('packages',{})
    if set(groups)-(B.OPEN_PACKAGES-B.PUBLIC_PACKAGES):raise MediaDrift('MEDIA_DRIFT unsupported official package')
    for package,ids in groups.items():
        if not ids or any(i not in seen for i in ids):raise MediaDrift('MEDIA_DRIFT incomplete official group: '+package)
    byid={e['id']:e for e in data['entries']}
    for version,key in data.get('driver_versions',{}).items():
        if key not in seen or byid[key].get('version')!=version or byid[key]['package']!='nvidia_driver':
            raise MediaDrift('MEDIA_DRIFT driver version does not match its pinned artifact')
    fetcher=data['fetcher']
    if fetcher.get('path')!='scripts/ci/kf_fetch.py' or not re.fullmatch(r'[0-9a-f]{64}',fetcher.get('sha256','')) or type(fetcher.get('size'))is not int or fetcher['size']<=0:
        raise MediaDrift('MEDIA_DRIFT invalid fetcher source pin')
    return data

def select(data,apps,driver_version=None):
    validate(data);groups=set()
    for app in apps:groups.update((app.get('requires')or{}).get('official_fetch',[]))
    if groups and not driver_version:raise MediaDrift('MEDIA_DRIFT explicit J2 Windows driver selection required')
    if groups-set(data['packages']):raise MediaDrift('MEDIA_DRIFT app requires an unpinned official package')
    ids={i for group in groups for i in data['packages'][group]}
    if driver_version:
        if driver_version not in data['driver_versions']:raise MediaDrift('MEDIA_DRIFT selected driver has no official pin')
        ids.add(data['driver_versions'][driver_version])
    return sorted(ids)

def driver_plan(data,driver_version):
    """J2 data interface: guest-only staging/install, BEFORE the Windows gate.

    The app harness checks the installed version and never changes a driver after
    its parent gate. No URL or version is inferred from guest-reported strings.
    """
    validate(data)
    if driver_version not in data['driver_versions']:raise MediaDrift('MEDIA_DRIFT selected driver has no official pin')
    entry=next(e for e in data['entries']if e['id']==data['driver_versions'][driver_version])
    dest=entry['guest_destination'];archive=str(PureWindowsPath(dest)/entry['name'])
    return dict(entry=entry['id'],version=driver_version,stage_package=entry['package'],
                guest_destination=dest,installer_argv=[x.replace('{dest}',dest).replace('{file}',archive)for x in entry['installer_argv']])

def check_fetcher(path,pin):
    path=Path(path)
    if not path.is_file() or path.is_symlink() or B.sha256(path)!=pin['sha256']:
        raise MediaDrift('MEDIA_DRIFT J4 fetcher absent or changed')
    if pin.get('size',path.stat().st_size)!=path.stat().st_size:raise MediaDrift('MEDIA_DRIFT J4 fetcher size changed')

def download(entry,destination,fetcher):
    env={k:os.environ[k]for k in ('PATH','SYSTEMROOT','WINDIR','TMPDIR','TEMP','TMP','SSL_CERT_FILE','SSL_CERT_DIR','LANG','LC_ALL','TZ')if k in os.environ}
    try:
        result=subprocess.run([sys.executable,'-I',str(fetcher),entry['official_url'],str(destination),'--sha256',entry['sha256'],'--size',str(entry['size']),'--streams','8','--follow-redirect'],env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,shell=False,timeout=3600)
    except (OSError,subprocess.TimeoutExpired):raise MediaDrift('MEDIA_DRIFT official fetch unavailable')from None
    if result.returncode:raise MediaDrift('MEDIA_DRIFT official input '+entry['id'].split('/')[0])
    try:B.verify_file(entry['package'],dict(name=entry['name'],sha256=entry['sha256'],size=entry['size']),destination)
    except ValueError:raise MediaDrift('MEDIA_DRIFT pinned bytes differ: '+entry['package'])from None
    print('OFFICIAL_OK package='+entry['package']+' bytes='+str(entry['size'])+' sha256='+entry['sha256'])

@contextmanager
def transient(data,ids,fetcher):
    """Local per-run media, deleted on normal exit AND errors; never a repo pin."""
    byid={e['id']:e for e in validate(data)['entries']}
    if not ids:
        yield None
        return
    check_fetcher(fetcher,data['fetcher'])
    if set(ids)-set(byid):raise MediaDrift('MEDIA_DRIFT unpinned official selection')
    with tempfile.TemporaryDirectory(prefix='kf-official-run-')as tmp:
        root=Path(tmp);files={};stage={};expected={}
        for index,key in enumerate(sorted(set(ids))):
            e=byid[key];dest=root/str(index);download(e,dest,fetcher)
            files['pkg/'+e['package']+'/'+e['name']]=dest
            expected['pkg/'+e['package']+'/'+e['name']]=dict(sha256=e['sha256'],size=e['size'])
            s=e.get('stage',dict(mode='copy',dest='official_driver'))
            row=stage.setdefault(e['package'],dict(mode=s['mode'],dest=s['dest'],strip_top=bool(s.get('strip_top')),run=s.get('run',[]),files=[]))
            row['files'].append(e['name'])
        files['stage.json']=B.canonical(stage)
        iso=root/'official-run.iso';actual=B.bundle_iso.write_image(files,iso,label='KFAPPSP')
        if any(actual[name]!=pin for name,pin in expected.items()):raise MediaDrift('MEDIA_DRIFT original changed during ISO assembly')
        manifest=dict(bundle_class='transient-official',files=actual,image=dict(sha256=B.sha256(iso),size=iso.stat().st_size,label='KFAPPSP'),entries=sorted(set(ids)))
        (root/'official-run.manifest.json').write_bytes(B.canonical(manifest))
        yield iso

def main(argv=None):
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=('check','verify'));p.add_argument('--fetcher',type=Path,default=HERE.parents[2]/'ci/kf_fetch.py');a=p.parse_args(argv)
    try:
        data=validate(B.load(HERE/'official_fetch.json'))
        if a.command=='verify':
            check_fetcher(a.fetcher,data['fetcher'])
            with tempfile.TemporaryDirectory(prefix='kf-official-verify-')as tmp:
                for i,e in enumerate(data['entries']):
                    dest=Path(tmp)/str(i);download(e,dest,a.fetcher);dest.unlink()
        print('OFFICIAL_'+a.command.upper()+'_OK');return 0
    except MediaDrift as error:
        print('OFFICIAL_FAIL class=MEDIA_DRIFT '+str(error),file=sys.stderr);return 3
    except (OSError,ValueError,KeyError,TypeError):
        print('OFFICIAL_FAIL class=MEDIA_DRIFT (details withheld)',file=sys.stderr);return 3

if __name__=='__main__':sys.exit(main())
