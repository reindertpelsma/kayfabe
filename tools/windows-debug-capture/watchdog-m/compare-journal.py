#!/usr/bin/env python3
"""Private, bounded L/M journal comparison. RVAs only; no driver bytes emitted."""
import argparse,collections,json,re
from pathlib import Path

def one(root):
 text=(root/'analysis.stdout').read_text(encoding='utf-8-sig',errors='strict')
 if len(text)>8*1024*1024:raise ValueError('Analysis output exceeds bound')
 match=re.search(r'^([0-9a-fA-F`]{16,17})\s+([0-9a-fA-F`]{16,17})\s+nvlddmkm\s',text,re.M)
 if not match:raise ValueError('No exact nvlddmkm loaded range')
 base,end=(int(x.replace('`',''),16) for x in match.groups())
 data=json.loads((root/'nvcd.json').read_text());assertions=[]
 def rva(address):return hex(address-base) if base<=address<end else 'outside-nvlddmkm'
 def walk(node):
  if isinstance(node,dict):
   for key,value in node.items():
    if key=='journal_assert':
     for item in value:
      assertions.append({'hint_rva':[rva(x) for x in item.get('breakpoint_addr_hint',[])],
                         'stack_rvas':[rva(x) for x in item.get('call_stack',[])],
                         'level':item.get('level',[])})
    walk(value)
  elif isinstance(node,list):
   for value in node:walk(value)
 walk(data.get('decoded',[]))
 return {'bugcheck_fields':re.findall(r'^BUGCHECK_(?:CODE|P[123]):\s*(\S+)',text,re.M),
         'nvcd_integrity':data.get('nvcd'),'assertions':assertions}

p=argparse.ArgumentParser(description=__doc__);p.add_argument('l',type=Path);p.add_argument('m',type=Path);a=p.parse_args()
l,m=one(a.l),one(a.m);prefix=0
for x,y in zip(l['assertions'],m['assertions']):
 if x!=y:break
 prefix+=1
print(json.dumps({'L':l,'M':m,'identical_assertion_prefix':prefix,
 'note':'Saved assertions and RVAs only. level is not NV_STATUS; missing/truncated journal bytes limit absence claims. Outside-module addresses are deliberately omitted.'},indent=2))
