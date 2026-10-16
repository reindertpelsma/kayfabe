from pathlib import Path
import csv,gzip,re,collections,json
base=Path(__file__).resolve().parents[1]
old=list(csv.DictReader((base/'virtual_native_alignment_20261016/virtual-api.tsv').open(),delimiter='\t'))
source=gzip.decompress((base/'virtual_native_alignment_20261016/qemu.log.gz').read_bytes()).decode()
rows=[];cursor=0;extra=[]
for line_number,line in enumerate(source.splitlines(),1):
 m=re.search(r'kf-rm: rpc-trace fn=(\d+) (.*?) seq=(\d+) (.*?) result=(0x[0-9a-f]+|none)\s*$',line)
 if not m:continue
 fn=int(m[1]);sel=re.search(r'(?:cmd|class)=(0x[0-9a-f]+)',m[4]);selector=hex(int(sel[1],16)) if sel else '-'
 if fn==47 or fn==4:
  assert selector=='-' and m[5]=='0x0'
  row={'function':str(fn),'kind':m[2],'rpc_sequence':m[3],'control_or_class':selector,'trace_result':m[5],'effective_result':m[5]}
  legacy='-';extra.append({'full_ordinal':len(rows),'function':fn,'line':line_number})
 else:
  assert cursor<len(old)
  row=dict(old[cursor]);legacy=row['ordinal'];cursor+=1
  assert fn==int(row['function']) and selector==row['control_or_class'] and m[5]==row['trace_result']
 row.update(ordinal=str(len(rows)),legacy_ordinal=legacy,source_line=str(line_number));rows.append(row)
 if fn==47:break
assert cursor==382 and len(rows)==385 and len(extra)==3
out=base/'offline_alignment_20261016'
with (out/'virtual-complete.tsv').open('w') as f:
 w=csv.DictWriter(f,['ordinal','legacy_ordinal','source_line','function','kind','rpc_sequence','control_or_class','trace_result','effective_result'],delimiter='\t',lineterminator='\n');w.writeheader();w.writerows(rows)
summary={'old_rows':len(old),'complete_rows':len(rows),'extra_calls':extra,'before_unload':rows[-2],'unload':rows[-1]}
(out/'virtual-complete-audit.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps(summary,indent=2))
