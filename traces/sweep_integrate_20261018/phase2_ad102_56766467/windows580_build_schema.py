from pathlib import Path
import re,json,subprocess
base=Path('/workspace/codex/headers5806506/src/common/sdk/nvidia/inc')
structs={};macros={};headers=[]
for p in (base/'ctrl/ctrl0073').glob('*.h'):
 headers.append(str(p.relative_to(base)));s=p.read_text()
 for m in re.finditer(r'#define\s+(NV0073_CTRL_CMD_\w+)\s+\((0x[0-9A-Fa-f]+)U?\)',s):macros[int(m[2],16)]=m[1]
 for m in re.finditer(r'typedef struct(?:\s+\w+)?\s*\{([^{}]*)\}\s*(\w+)\s*;',s,re.S):structs[m[2]]=m[1]
headers.append('ctrl/ctrl2080/ctrl2080internal.h')
s=(base/headers[-1]).read_text()
m=re.search(r'typedef struct NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS\s*\{(.*?)\}\s*NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS;',s,re.S);structs['NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS']=m[1]
macros[0x20800a58]='NV2080_CTRL_CMD_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER'
code=['#include <stdio.h>','#include <stddef.h>']+['#include "'+h+'"' for h in headers]+['int main(void){']
meta={}
for key,macro in macros.items():
 if key not in [int(k,16) for k in ['730151','730107','73028b','73013d','730101','731369','73014b','730102','730287','730108','730240','730250','731140','73028a','730116','730245','73010b','73012c','730109','20800a58','730301','73010c','731142','73010a','731152','7302a4']]:continue
 typ=next((t for t in [macro.replace('_CMD_','_')+'_PARAMS',macro+'_PARAMS'] if t in structs),None)
 if not typ:continue
 body=re.sub(r'/\*.*?\*/|//[^\n]*','',structs[typ],flags=re.S);fields=[]
 for decl in body.split(';'):
  decl=decl.strip()
  if not decl:continue
  m=re.fullmatch(r'(NvU8|NvU16|NvU32|NvU64|NvS32|NvBool|NvHandle|NvP64)\s+(\w+)(?:\[[^;{}]+\])?',decl)
  if not m:m=re.fullmatch(r'NV_DECLARE_ALIGNED\((NvU64|NvP64)\s+(\w+),\s*8\)',decl)
  if m:fields.append((m[1],m[2]))
 if not fields:continue
 meta[str(key)]={'type':typ,'fields':{n:{'ctype':t} for t,n in fields},'abi':'580.65.06'}
 code.append(f'printf("SIZE {key} %zu\\n",sizeof({typ}));')
 for t,n in fields:code.append(f'printf("FIELD {key} {n} %zu %zu\\n",offsetof({typ},{n}),sizeof((({typ}*)0)->{n}));')
code.append('return 0;}')
c=Path('/workspace/codex/sweep-display-layout.c');c.write_text('\n'.join(code)+'\n')
subprocess.run(['gcc','-std=gnu11','-I',str(base),str(c),'-o','/workspace/codex/sweep-display-layout'],check=True)
raw=subprocess.check_output(['/workspace/codex/sweep-display-layout'],text=True)
Path('/workspace/codex/sweep-display-layout.txt').write_text(raw)
for row in raw.splitlines():
 f=row.split();m=meta[f[1]]
 if f[0]=='SIZE':m['size']=int(f[2])
 else:m['fields'][f[2]].update(offset=int(f[3]),bytes=int(f[4]))
meta['_provenance']={'abi':'580.65.06','control_params_at':40,'source':'public NVIDIA580.65.06 headers; compiler sizeof/offsetof; partial fields only; no nested unknown fields','omissions':'unknown/serialized/truncated controls and unknown fields omitted;64bit/pointer/address/handle fields redacted'}
Path('/workspace/codex/sweep-display-schema580.json').write_text(json.dumps(meta,indent=2)+'\n')
print('PUBLIC_COMPILER_SCHEMA controls='+str(len(meta)-1)+' pushbuffer_size='+str(meta[str(0x20800a58)]['size']))
