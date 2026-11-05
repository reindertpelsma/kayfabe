#!/usr/bin/env python3
"""cmp_precomp.py <new precomp_tier3.txt> <table tsv> <table die> [<ref capture txt>]
Words of NEW vs the committed table rows of <table die>; words+fields vs a reference capture (optional)."""
import sys
def cap(path):
    words, fields, tags = {}, {}, set()
    for line in open(path):
        p = line.rstrip("\n").split("\t")
        if len(p) < 3 or p[1] not in ("word", "field"):
            continue
        tags.add(p[0]); kv = dict(x.split("=", 1) for x in p[2:])
        if p[1] == "word":
            words[int(kv["address"], 16)] = int(kv["value"], 0)
        else:
            fields[(kv["window"], kv["name"])] = int(kv["value"], 0)
    return words, fields, tags
new_w, new_f, new_t = cap(sys.argv[1])
tab = {}
for line in open(sys.argv[2]):
    if line.startswith("#") or line.startswith("key\t"): continue
    p = line.rstrip("\n").split("\t")
    if len(p) >= 6 and p[1] == sys.argv[3]:
        tab[0x640000 + int(p[0].rsplit(".", 1)[1], 16)] = int(p[5], 0)
diff = [a for a in sorted(set(tab) | set(new_w)) if tab.get(a) != new_w.get(a)]
print(f"NEW {sorted(new_t)} words {len(new_w)} fields {len(new_f)}")
print(f"TABLE die={sys.argv[3]} rows {len(tab)}")
for a in diff: print(f"  DIFF word 0x{a:08x} table={tab.get(a)} new={new_w.get(a)}")
print(f"PRECOMP_VS_TABLE die={sys.argv[3]} words_differ={len(diff)} rows_table={len(tab)} words_new={len(new_w)}")
if len(sys.argv) > 4:
    rw, rf, rt = cap(sys.argv[4])
    wd = [a for a in set(rw) | set(new_w) if rw.get(a) != new_w.get(a)]
    fd = [k for k in set(rf) | set(new_f) if rf.get(k) != new_f.get(k)]
    for k in sorted(fd)[:20]: print(f"  DIFF field {k} ref={rf.get(k)} new={new_f.get(k)}")
    print(f"REF {sorted(rt)} words {len(rw)} fields {len(rf)}")
    print(f"PRECOMP_COMPARE words_differ={len(wd)} fields_differ={len(fd)} fields_ref={len(rf)} fields_new={len(new_f)}")
