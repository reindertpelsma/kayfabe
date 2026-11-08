#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Generate the official run-time catalogue/lock from existing exact vendor pins."""
import json
from pathlib import Path
import sys

HERE=Path(__file__).resolve().parent

def catalogue(manifest):
    entries=[];packages={}
    for p in manifest['packages']:
        if p.get('class')!='official-fetch':continue
        packages[p['id']]=[]
        for f in p['files']:
            key=p['id']+'/'+f['name'];packages[p['id']].append(key)
            entries.append(dict(id=key,package=p['id'],name=f['name'],official_url=f['url'],sha256=f['sha256'],size=f['size'],
                license_id=p['compliance'].get('components',{}).get(f['name'],{}).get('license_id',p['compliance']['license_id']),
                guest_destination='C:\\kfapps\\'+p['stage']['dest'].replace('/','\\'),installer_argv=p['stage'].get('run',[]),
                stage=p['stage'],**{'class':'official-fetch'}))
    drivers={}
    for pin in manifest['official_drivers']:
        e=dict(pin);e['official_url']=e.pop('url');e['class']='official-fetch';entries.append(e);drivers[e['version']]=e['id']
    return dict(schema=1,status='PINNED, 2026-11-08; official downloads only, per-run use; no redistribution',
                fetcher=manifest['official_fetcher'],entries=sorted(entries,key=lambda e:e['id']),packages=packages,driver_versions=drivers)

def lock(data):
    lines=['# Hash-locked direct wheel artifacts. No pip resolution is performed.']
    for e in data['entries']:
        if e['name'].endswith('.whl'):
            name=e['name'].split('-')[0].replace('_','-')
            lines.append(name+' @ '+e['official_url']+' --hash=sha256:'+e['sha256'])
    return '\n'.join(lines)+'\n'

def main(argv=None):
    data=catalogue(json.loads((HERE/'manifest.json').read_text()))
    outputs={HERE/'official_fetch.json':json.dumps(data,indent=1)+'\n',HERE/'official_requirements.txt':lock(data)}
    check='--check'in(argv if argv is not None else sys.argv[1:])
    if check:return int(any(not p.is_file()or p.read_text()!=text for p,text in outputs.items()))
    for p,text in outputs.items():p.write_text(text)
    return 0

if __name__=='__main__':sys.exit(main())
