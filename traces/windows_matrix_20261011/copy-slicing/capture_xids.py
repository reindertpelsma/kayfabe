import datetime,json,pathlib,subprocess,sys
out=pathlib.Path(sys.argv[1]); out.mkdir(parents=True,exist_ok=True)
start=sys.argv[2]
end=datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%d %H:%M:%S UTC')
p=subprocess.run(['journalctl','-k','--since',start,'--until',end,'--utc','--no-pager','-o','short-iso'],capture_output=True,text=True)
lines=[s for s in p.stdout.splitlines() if 'NVRM: Xid' in s]
(out/'kernel-xids.txt').write_text('\n'.join(lines)+ ('\n' if lines else ''))
meta={'since':start,'until':end,'source':'kernel journal','query_exit':p.returncode,'xid_records':len(lines) if p.returncode==0 else None,'query_stderr':p.stderr.strip()}
(out/'kernel-xids.json').write_text(json.dumps(meta,indent=2)+'\n')
print('KERNEL_JOURNAL_XID_RECORDS',meta['xid_records'],'QUERY_EXIT',p.returncode)
raise SystemExit(p.returncode)
