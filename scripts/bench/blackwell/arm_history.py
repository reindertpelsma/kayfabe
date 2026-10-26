"""Workloads need a fault-free current arm, even after an earlier diagnostic arm."""
def may_run_apps(native, start, current):
    return tuple(start[:len(native)])==tuple(native) and tuple(current)==tuple(start)

def main():
    import json,sys
    from pathlib import Path
    def read(path):
        with Path(path).open() as f:s=f.read(65537)
        if len(s)>65536:raise ValueError('history byte bound')
        return json.loads(s)
    def lines(value):
        if not isinstance(value,list) or not all(isinstance(x,str) for x in value):raise ValueError('invalid history')
        return value
    try:
        mode,native_path,start_path,current_path=sys.argv[1:]
        native=lines(read(native_path)['approved_native_xid_lines'])
        if mode=='snapshot':
            current=lines(read(start_path)['approved_native_xid_lines'])
            if current[:len(native)]!=native:raise ValueError('native prefix changed')
            p=Path(current_path)
            if p.exists():raise ValueError('snapshot already exists')
            p.write_text(json.dumps({'accepted_lines':current})+'\n');return 0
        if mode!='check':raise ValueError('unknown mode')
        start=lines(read(start_path)['accepted_lines']);current=lines(read(current_path)['approved_native_xid_lines'])
        if start[:len(native)]!=native or current[:len(start)]!=start:raise ValueError('immutable history lost/replaced')
        if may_run_apps(native,start,current):return 0
        print('ARM_DIAGNOSTIC_FAULTS no validation workloads in this arm');return 1
    except (OSError,ValueError,KeyError,TypeError) as error:
        print('ARM_HISTORY_INVALID_STOP '+str(error),file=sys.stderr);return 2

if __name__=='__main__':raise SystemExit(main())
