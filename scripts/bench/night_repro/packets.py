#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Pair DxgKrnl command-buffer 178/180 by first/third payload field.

Unmatched starts are pending at the captured trace boundary, not proof that a
packet never completed afterward. Execution PID is retained without claiming
it owns the GPU context (kernel submissions may have a different owner).
"""
import argparse
import json
import xml.etree.ElementTree as ET


def pair(events):
    pending={}
    submitted=completed=unmatched_stop=duplicate_start=0
    for event,context,sequence,pid,timestamp in events:
        key=(context, sequence)
        if event==178:
            submitted+=1
            duplicate_start+=int(key in pending)
            pending[key]=dict(context=context,sequence=sequence,execution_pid=pid,time=timestamp)
        elif event==180:
            if pending.pop(key,None) is not None:
                completed+=1
            else:
                unmatched_stop+=1
    return dict(submitted=submitted,completed=completed,unmatched_stop=unmatched_stop,
                duplicate_start=duplicate_start,pending=list(pending.values()))


def xml_events(path):
    # tracerpt XML uses the Windows Event schema, with or without namespaces.
    for _,event in ET.iterparse(path,events=('end',)):
        if event.tag.rsplit('}',1)[-1]!='Event':
            continue
        fields={node.tag.rsplit('}',1)[-1]:node for node in event.iter()}
        eid=fields.get('EventID')
        payload=fields.get('EventData')
        if eid is not None and eid.text in ('178','180') and payload is not None:
            data=[n.text or '' for n in payload if n.tag.rsplit('}',1)[-1]=='Data']
            if len(data)>=3:
                execution=fields.get('Execution')
                created=fields.get('TimeCreated')
                yield (int(eid.text),data[0],data[2],
                    int(execution.attrib.get('ProcessID',0)) if execution is not None else 0,
                    created.attrib.get('SystemTime','') if created is not None else '')
        event.clear()


if __name__=='__main__':
    p=argparse.ArgumentParser()
    p.add_argument('xml')
    args=p.parse_args()
    print(json.dumps(pair(xml_events(args.xml)),indent=2))
