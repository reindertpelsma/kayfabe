"""GPU-free audit of published layouts and authored output. No native capture is read."""
from pathlib import Path
import hashlib,json,re,csv
ROOT=Path(__file__).resolve().parents[3]
OUT=Path(__file__).resolve().parent
PREFIX='NVCA73_PRECOMP_WIN_PIPE_HDR_'
versions=json.loads((OUT.parent/'abi/sources.json').read_text())
layouts={}
for version in versions:
 p=ROOT/'crates/kf-disp/data'/('classes-'+version+'.tsv')
 fields={}
 for line in p.read_text().splitlines():
  f=line.split('\t')
  if f[0]=='F' and f[1].startswith(PREFIX):
   register,field=f[1][len(PREFIX):].split('_',1)
   fields[(register,field)]=(int(f[2]),int(f[3]))
 assert len(fields)==41,(version,len(fields))
 layouts[version]={'fields':fields,'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
first=next(iter(layouts.values()))['fields']
assert all(x['fields']==first for x in layouts.values())
# This is the checked-in DERIVED family report, not a native page.
words={}
p=ROOT/'crates/kf-disp/data/caps_family_pages.tsv'
for line in p.read_text().splitlines():
 f=line.split('\t')
 if len(f)>=4 and f[0]=='0xCA73' and f[2]!='-':
  assert f[3]=='Rule',f[:4]
  words[int(f[1],16)]=int(f[2],16)
known={
 'CAPA':{'CSC00_PRESENT','CSC01_PRESENT','CSC0LUT_PRESENT','CSC10_PRESENT','CSC11_PRESENT','CSC1LUT_PRESENT','SCLR_PRESENT','TMO_PRESENT'},
 'CAPB':{'ILUT_LOGSZ','ILUT_LOGNR','ILUT_DIRECT','ILUT_SFCLOAD'},
 'CAPC':{'CSC00_PRECISION','CSC01_PRECISION'},
 'CAPD':{'TMO_LOGSZ','TMO_LOGNR','TMO_SFCLOAD'},
 'CAPE':set(),'CAPF':set(),
}
unclaimed={
 'CAPA':{'GMA_PRESENT'},'CAPB':set(),
 'CAPC':{'CSC00_UNITY_CLAMP','CSC01_UNITY_CLAMP','CSC0LUT_DIRECT','CSC0LUT_FVLUT'},
 'CAPD':{'TMO_DIRECT'},
 'CAPE':{'CSC10_UNITY_CLAMP','CSC11_UNITY_CLAMP','CSC1LUT_DIRECT','CSC1LUT_FVLUT'},'CAPF':set(),
}
rows=[]
for (register,field),(hi,lo) in sorted(first.items()):
 idx=ord(register[-1])-ord('A');base=0x780+idx*4;mask=(1<<(hi-lo+1))-1
 values=[(words.get(base+i*32,0)>>lo)&mask for i in [0,1]]
 if field in known[register]:
  policy='T1-authored-candidate' if register=='CAPC' else 'T1-authored'
  reason='inferred virtual S5.14 declaration; native meaning/numeric fidelity not proved' if register=='CAPC' else ('existing virtual LUT presence claim; CA curve behavior is not verified' if field in {'CSC0LUT_PRESENT','CSC1LUT_PRESENT'} else 'existing virtual engine/topology/SDR contract; not a physical die fact')
 elif field in unclaimed[register]:
  policy='T1-unclaimed'
  reason='unsupported feature not advertised; native TRUE/behavior not inferred from zero'
  assert values==[0,0],(register,field,values)
 else:
  policy='unresolved'
  reason='no explicit implemented rule here; current zero is not a proven physical value'
 rows.append({'register':register,'field':field,'bits':str(hi)+':'+str(lo),'current_T1_main':values[0],'current_T1_overlay':values[1],'design_tier':policy,'why':reason,'physical_T2':'candidate query only; offset permission n=0','current_PR85_success_path':'whole-word T2 override; HELD pending field-level correction'})
with (OUT/'precomp_fields.tsv').open('w',newline='') as f:
 w=csv.DictWriter(f,fieldnames=list(rows[0]),delimiter='\t',lineterminator='\n');w.writeheader();w.writerows(rows)
receipt={'scope':'CODE READ only: no native capture replay/import and no hardware access','public_layout_versions':{k:{'commit':versions[k]['commit'],'generated_classes_sha256':v['sha256']} for k,v in layouts.items()},'public_fields_count':len(rows),'all_five_layouts_identical':True,'authored_family_report_sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'field_policies':{k:sum(r['design_tier']==k for r in rows) for k in sorted(set(r['design_tier'] for r in rows))},'caveat':'Per-field values decoded from a checked-in authored report are CODE READ, not a fresh build, physical measurement or acceptance result.'}
(OUT/'inventory.json').write_text(json.dumps(receipt,indent=2)+'\n')
print(json.dumps(receipt,indent=2))
