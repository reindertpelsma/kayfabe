from pathlib import Path
import importlib.util
import sys
spec=importlib.util.spec_from_file_location('collapse','tools/drivermatrix/collapse.py')
c=importlib.util.module_from_spec(spec);spec.loader.exec_module(c)
if len(sys.argv) != 2:
    raise SystemExit('usage: python3 compare_matrix.py FULL_COMPILER_SWEEP')
tags,lay,val,mis=c.load(sys.argv[1])
# Supplement the full sweep with the newly compiled timer structure. The two
# inventories must agree wherever they overlap; never silently replace a cell.
for tag in tags:
    timer = Path('traces/windows_timer_20261005/matrix') / tag / 'layouts.tsv'
    extra = {}
    for line in timer.read_text().splitlines():
        name, field, off, size, ty = line.split('\t')
        extra.setdefault(name, []).append((field, int(off), int(size), ty))
    for name, rows in extra.items():
        if name in lay[tag] and lay[tag][name] != rows:
            raise SystemExit(f'conflicting compiler measurements: {tag} {name}')
        lay[tag][name] = rows
rows=Path('traces/driver_matrix/ranges.tsv').read_text().splitlines()
oldtags=set(rows[1].split('\t')[1].split())
measured=[t for t in tags if t in oldtags]
if set(measured) != oldtags:
    raise SystemExit(f'missing measured tags: {sorted(oldtags - set(measured))}')
fields={t:{n:c.fields(r) for n,r in lay[t].items()} for t in measured}
elems={t:{n:c.elems(r) for n,r in lay[t].items()} for t in measured}
checked=0;errors=[]
for row in rows:
 if row.startswith('#'): continue
 kind,item,first,last,expected=row.split('\t')
 for t in measured:
  if not (c.vkey(first)<=c.vkey(t)<=c.vkey(last)): continue
  if kind=='value': actual=val[t].get(item,'ABSENT')
  else:
   name,field=item.split('.',1) if kind=='field' else (item,'.')
   got=fields[t].get(name,{}).get(field)
   actual='ABSENT' if got is None else f'{got[0]}+{got[1]}'
   e=elems[t].get(name,{}).get(field,0)
   if got is not None and e: actual+=f'@{e}'
  checked+=1
  if str(actual)!=expected: errors.append((t,kind,item,expected,actual))
print('MEASURED_TAGS_CHECKED',len(measured),'CELLS',checked,'DIFFERENCES',len(errors))
for err in errors[:30]: print(err)
raise SystemExit(bool(errors))
