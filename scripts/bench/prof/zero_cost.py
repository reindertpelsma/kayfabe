#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Assembly-free audit of trusted, locally built feature-off archives."""
import argparse,hashlib,json,re,subprocess
from pathlib import Path

def _symbols(text):
    for line in text.splitlines():
        m=re.match(r'^\s*(?:[0-9a-fA-F]+\s+)?([A-Z])\s+(.+)$',line)
        if m and m[1]!='U':yield m[1],m[2]
def profiler_symbols(text):return sorted({name for _,name in _symbols(text)if 'kf_prof' in name})
def public_symbols(text):return {kind+' '+re.sub(r'::h[0-9a-f]{16}(?=$|[. >)])','',name)for kind,name in _symbols(text)}
def inspect(path):
    p=Path(path)
    nm=subprocess.check_output(['nm','-g','-C','--defined-only',str(p)],text=True,stderr=subprocess.DEVNULL,timeout=60)
    output=subprocess.check_output(['size','-t',str(p)],text=True,timeout=60)
    totals=next((line for line in reversed(output.splitlines())if '(TOTALS)'in line),None)
    if totals is None:raise ValueError('archive section totals unavailable')
    fields=totals.split();sections=dict(zip(['text','data','bss'],map(int,fields[:3])))
    digest=hashlib.sha256()
    with p.open('rb')as stream:
        while chunk:=stream.read(1024*1024):digest.update(chunk)
    return dict(bytes=p.stat().st_size,sha256=digest.hexdigest(),sections=sections,public_symbols=len(public_symbols(nm)),profiler_symbols=profiler_symbols(nm)),public_symbols(nm)
def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('base');p.add_argument('current');args=p.parse_args()
    base,bsyms=inspect(args.base);current,csyms=inspect(args.current)
    if base['profiler_symbols']or current['profiler_symbols']:p.exit(1,'feature-off archive contains profiler definitions\n')
    print(json.dumps(dict(base=base,current=current,section_deltas={k:current['sections'][k]-base['sections'][k]for k in base['sections']},added_public_symbols=len(csyms-bsyms),removed_public_symbols=len(bsyms-csyms),scope='archive comparison; source paths and panic metadata can change size; no hardware overhead claim'),indent=2))
if __name__=='__main__':main()
