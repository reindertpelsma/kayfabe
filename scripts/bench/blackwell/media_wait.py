"""Checksum-pinned preparation waits for success, bounded by the lease budget."""
class MediaRefused(RuntimeError):pass

def await_media(ready, alive, clock, sleep, deadline):
    while clock()<=deadline:
        if ready():return True
        if not alive():raise MediaRefused('media job exited without success')
        remaining=deadline-clock()
        if remaining<=0:break
        sleep(min(5,remaining))
    raise MediaRefused('media preparation budget expired')

def main():
    import sys,json,os,time,re,datetime
    from pathlib import Path
    lease=json.loads(Path(sys.argv[1]).read_text());log=Path(sys.argv[2]);pid=int(Path(sys.argv[3]).read_text());assert pid>1
    created=datetime.datetime.fromisoformat(lease['created_utc'])
    end=created+datetime.timedelta(minutes=min(lease['auto_destroy_minutes'],lease['budget_cutoff_minutes']))
    deadline_utc=min(created+datetime.timedelta(minutes=40),end-datetime.timedelta(minutes=65))
    deadline=time.monotonic()+max(0,(deadline_utc-datetime.datetime.now(datetime.timezone.utc)).total_seconds())
    def ready():
        with log.open('rb') as f:
            f.seek(max(0,log.stat().st_size-4096));tail=f.read(4096).decode(errors='replace')
        return bool(re.search(r'^WIN_MEDIA_EXIT rc=0 ',tail,re.M))
    def alive():
        try:os.kill(pid,0);return True
        except ProcessLookupError:return False
    await_media(ready,alive,time.monotonic,time.sleep,deadline)
    print('MEDIA_SUCCESS_WITHIN_PREPARATION_BUDGET '+datetime.datetime.now(datetime.timezone.utc).isoformat())

if __name__=='__main__':main()
