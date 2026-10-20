#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""H1 context events. Snapshots provide no frame identity or causal coverage."""
import argparse,json
import reader

def context_records(rows):
    out=[]
    for row in rows:
        checked=reader._json(json.dumps(row,allow_nan=False).encode())
        out.append(dict(kind='prof_snapshot',ts_ns=checked['start_ns'],end_ns=checked['end_ns'],prof=checked,full_causal_coverage=False))
    return out

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('file');args=p.parse_args()
    try:
        for event in context_records(reader.load(args.file)['records']):print(json.dumps(event,allow_nan=False))
    except (OSError,ValueError) as e:p.exit(2,f'kf-prof H1 export: {e}\n')
if __name__=='__main__':main()
