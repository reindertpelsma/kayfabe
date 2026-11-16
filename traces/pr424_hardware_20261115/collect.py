import glob, os, re, json, sys
PATS = {
 "cut": re.compile(r"walk report CUT"),
 "truncated": re.compile(r"walk report TRUNCATED"),
 "slow_unmap": re.compile(r"SLOW host unmap call"),
 "split_late": re.compile(r"SPLIT-LATE"),
 "cap_ceiling": re.compile(r"capacity ceiling"),
 "dead": re.compile(r"chan token .*DEAD"),
 "dead_any": re.compile(r"\bDEAD\b"),
 "no_run_applied": re.compile(r"no run of its window applied"),
 "budget": re.compile(r"windows of a diff larger than the walk report"),
 "slow_apply": re.compile(r"SLOW-APPLY"),
 "large_apply": re.compile(r"mem large apply"),
}
files = sorted(glob.glob("/workspace/bench/*.log") + glob.glob("/root/prov/*.log") + glob.glob("/workspace/*.log") + glob.glob("/workspace/bench/*/*.log") + glob.glob("/workspace/winvm/kfwin/logs/*.log") + glob.glob("/workspace/winvm/*/*.log"))
out = sys.argv[1]
os.makedirs(out, exist_ok=True)
rows = []
lines_out = {k: [] for k in PATS}
for f in files:
    if f.endswith("payload_stdout.log") and False: continue
    try: data = open(f, errors="replace").read().splitlines()
    except OSError: continue
    cnt = {k: 0 for k in PATS}
    for ln in data:
        for k, p in PATS.items():
            if p.search(ln):
                cnt[k] += 1
                if len(lines_out[k]) < 4000: lines_out[k].append(os.path.basename(f) + ": " + ln[:400])
    if any(cnt.values()): rows.append((f, cnt))
with open(out + "/pattern_counts.tsv", "w") as w:
    w.write("file\t" + "\t".join(PATS) + "\n")
    for f, c in rows: w.write(os.path.basename(f) + "\t" + "\t".join(str(c[k]) for k in PATS) + "\n")
for k, v in lines_out.items():
    open(out + "/lines_" + k + ".txt", "w").write("\n".join(v) + ("\n" if v else ""))
# derived numbers
ms = [int(m.group(1)) for l in lines_out["slow_unmap"] for m in [re.search(r"entr\(ies\) in (\d+) ms", l)] if m]
cut_runs = [(int(m.group(1)), int(m.group(2))) for l in lines_out["cut"] for m in [re.search(r"at its capacity \((\d+) of (\d+)\)", l)] if m]
sl = [int(m.group(1)) for l in lines_out["split_late"] for m in [re.search(r" ms=(\d+)", l)] if m]
summ = dict(cut_lines=len(lines_out["cut"]), truncated_lines=len(lines_out["truncated"]), slow_unmap_lines=len(ms),
            slow_unmap_max_ms=max(ms) if ms else None, split_late_lines=len(sl), split_late_max_ms=max(sl) if sl else None,
            cap_ceiling_lines=len(lines_out["cap_ceiling"]), dead_lines=len(lines_out["dead"]),
            no_run_applied=len(lines_out["no_run_applied"]), budget_failures=len(lines_out["budget"]),
            cut_report_runs_max=max((a for a, b in cut_runs), default=None))
json.dump(summ, open(out + "/summary.json", "w"), indent=1)
print(json.dumps(summ))
