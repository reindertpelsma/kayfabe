#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Scoped syntactic wait audit. Does not claim type resolution or full-crate coverage."""
import re
from pathlib import Path

def tokens(source):
    out=[];i=0
    while i<len(source):
        if source.startswith('//',i):
            n=source.find('\n',i);i=len(source) if n<0 else n;continue
        if source.startswith('/*',i):
            depth=1;i+=2
            while i<len(source) and depth:
                if source.startswith('/*',i):depth+=1;i+=2
                elif source.startswith('*/',i):depth-=1;i+=2
                else:i+=1
            continue
        # Rust strings, including raw strings, are not executable expressions.
        raw=re.match(r'(?:b)?r(#+)?"',source[i:])
        if raw:
            marker='"'+(raw.group(1) or '');i+=raw.end();end=source.find(marker,i);i=len(source) if end<0 else end+len(marker);continue
        char=re.match(r"'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]+\}|.)|[^'\\\n])'",source[i:])
        if char:i+=char.end();continue
        if source[i]=='"':
            i+=1
            while i<len(source):
                c=source[i];i+=1
                if c=='\\':i+=1
                elif c=='"':break
            continue
        name=re.match(r'(?:r#)?([A-Za-z_][A-Za-z_0-9]*)',source[i:])
        if name:out.append((name.group(1),i));i+=name.end()
        elif source[i].isspace():i+=1
        else:out.append((source[i],i));i+=1
    return out

def unwrapped(source):
    ts=tokens(source);stack=[];bad=[]
    for i,(t,off) in enumerate(ts):
        if t in ('(','[','{'):
            wrapped=t=='(' and i>=2 and ts[i-1][0]=='!' and ts[i-2][0]=='prof_wait'
            stack.append(wrapped or (stack[-1] if stack else False))
        elif t in (')',']','}'):
            if stack:stack.pop()
        elif t=='wait' and i>0 and (ts[i-1][0]=='.' or (i>=2 and ts[i-1][0]==ts[i-2][0]==':')) and i+1<len(ts) and ts[i+1][0]=='(':
            if not (stack and stack[-1]):bad.append(source.count('\n',0,off)+1)
    return bad

def main():
    root=Path(__file__).resolve().parents[3]
    path=root/'crates/kf-chan/src/worker.rs';bad=unwrapped(path.read_text())
    if bad:raise SystemExit(f'{path.relative_to(root)}: unprofiled waits at lines {bad}')
    print('PASS: channel worker poll expressions; other files are outside this scoped gate')
if __name__=='__main__':main()
