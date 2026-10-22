#!/usr/bin/env python3
"""Extract measured SPEC subtest FPS; preserve failed/incomplete workload status.

Input: text-only guest export JSON {root: ..., files: [{path: ..., text: ...}]}.
No composite is inferred from a partial XML file. Vendor CSV composites should
be collected separately with the workload's completion status.
"""
import argparse
import csv
import json
import math
from pathlib import Path, PurePosixPath
import re
import sys
import xml.etree.ElementTree as ET


def parse_stdout(text):
    rows=[]
    for block in re.findall(r'<Results\b.*?</Results>',text,re.DOTALL):
        root=ET.fromstring(block)
        for test in root.findall('Test'):
            index=int(test.attrib['Index'])
            fps=float(test.attrib['FPS_'+str(index)])
            if not math.isfinite(fps) or fps <= 0:
                raise ValueError('invalid measured FPS')
            rows.append({'workload':root.attrib['Name'],'subtest':index,'fps':fps,
                         'frames':int(test.attrib['Frames']),'seconds':float(test.attrib['Seconds'])})
    return rows


def parse_statuses(text):
    return dict(re.findall(r'\[Workload:\s*([a-z0-9_-]+)\]\s+(Completed|Failed)\b',text))



def scores_from_export(data, arm):
    statuses = {}
    fallback = PurePosixPath(data['root'].replace('\\', '/')).name
    for record in data['files']:
        parts = PurePosixPath(record['path'].replace('\\', '/')).parts
        if parts[-1] == 'run-log.txt' and (len(parts) == 1 or parts[-2].startswith('result_')):
            run = next((part for part in parts if part.startswith('result_')), fallback)
            statuses[run] = parse_statuses(record['text'])
    rows = []
    for score in data.get('scores', []):
        status = statuses.get(score['run'], {}).get(score['workload'], 'Incomplete')
        reported = score.get('composite')
        qualified = None
        if status == 'Completed' and reported is not None:
            qualified = float(reported)
            if not math.isfinite(qualified) or qualified <= 0:
                raise ValueError('invalid completed workload composite')
        rows.append({'arm': arm, 'run': score['run'], 'workload': score['workload'],
                     'composite': qualified, 'reported_composite': reported,
                     'workload_status': status, 'source': score.get('source', 'result.json'),
                     'version': score.get('version', '')})
    return rows


def rows_from_export(data,arm):
    fallback=PurePosixPath(data['root'].replace('\\','/')).name
    statuses={}
    def identify(path):
        parts=PurePosixPath(path.replace('\\','/')).parts
        run=next((p for p in parts if p.startswith('result_')),fallback)
        return parts,run
    for record in data['files']:
        parts,run=identify(record['path'])
        if parts[-1]=='run-log.txt' and (len(parts)==1 or parts[-2].startswith('result_')):
            statuses[run]=parse_statuses(record['text'])
    rows=[]
    for record in data['files']:
        parts,run=identify(record['path'])
        if parts[-1]!='stdout.log':
            continue
        for row in parse_stdout(record['text']):
            rows.append({'arm':arm,'run':run,**row,
                         'workload_status':statuses.get(run,{}).get(row['workload'],'Incomplete')})
    return rows


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('export',type=Path)
    p.add_argument('--arm',required=True)
    p.add_argument('--output',type=Path)
    p.add_argument('--scores-output',type=Path)
    a=p.parse_args()
    data=json.loads(a.export.read_text(encoding='utf-8-sig'))
    rows=rows_from_export(data,a.arm)
    if a.scores_output:
        with a.scores_output.open('w', newline='') as scores_file:
            writer=csv.DictWriter(scores_file,fieldnames=['arm','run','workload','composite','reported_composite','workload_status','source','version'],lineterminator='\n')
            writer.writeheader();writer.writerows(scores_from_export(data,a.arm))
    fields=['arm','run','workload','subtest','fps','frames','seconds','workload_status']
    stream=a.output.open('w',newline='') if a.output else sys.stdout
    try:
        writer=csv.DictWriter(stream,fieldnames=fields,lineterminator='\n')
        writer.writeheader();writer.writerows(rows)
    finally:
        if a.output:stream.close()


if __name__=='__main__':main()
