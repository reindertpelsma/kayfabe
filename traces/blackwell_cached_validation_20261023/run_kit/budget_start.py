from pathlib import Path
import datetime,json,sys
j=json.loads(Path('/root/sweep2/lease.json').read_text());now=datetime.datetime.now(datetime.timezone.utc)
end=datetime.datetime.fromisoformat(j['created_utc'])+datetime.timedelta(minutes=min(j['auto_destroy_minutes'],j['budget_cutoff_minutes'])-25)
r={'cell':sys.argv[1],'utc':now.isoformat(),'latest_start_utc':end.isoformat(),'budget_cutoff_minutes':j['budget_cutoff_minutes'],'may_start':now<end}
Path('/root/prov/budget_start_'+sys.argv[1]+'.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r));sys.exit(0 if r['may_start'] else 30)
