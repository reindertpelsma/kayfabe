#!/usr/bin/env python3
"""Host-side fixed-file QGA transfer. No VM/GPU lifecycle operations.

Use serially through the existing pinned PC qmp.py. stage sends two trusted
controller files; fetch retrieves only the prepared private M export, <=8MiB/file.
"""
import argparse, base64, hashlib, importlib.util, json, os
from pathlib import Path

LIMIT=8*1024*1024
CHUNK=48*1024
NAMES={'watchdog.dmp','dump-source.json','wrapper.stdout','analysis.stdout',
       'analysis.stderr','metadata.json','result.json','commands.txt','receipt.json'}
TRUSTED={'analyze-dump.ps1':'7373e87e3f2681ccd223ada92f1c561e8f29b147c83c599d3865aa1a2049dc6a',
         'kf-kd-bundle.zip':'a3883456c4631bb46b9091a2d3197f17984df99ba89c36c2f8e54c976052d800'}

def main():
 p=argparse.ArgumentParser(description=__doc__)
 p.add_argument('--helper',type=Path,required=True)
 p.add_argument('--socket',type=Path,required=True)
 p.add_argument('action',choices=['stage','fetch'])
 p.add_argument('directory',type=Path)
 a=p.parse_args(); os.umask(0o077)
 spec=importlib.util.spec_from_file_location('existing_qmp',a.helper)
 q=importlib.util.module_from_spec(spec);spec.loader.exec_module(q)
 if a.action=='stage':
  blobs={name:(a.directory/name).read_bytes() for name in TRUSTED}
  for name,data in blobs.items():
   if len(data)>32*1024*1024 or hashlib.sha256(data).hexdigest()!=TRUSTED[name]:
    raise ValueError('Untrusted stage artifact: '+name)
 else:
  a.directory.mkdir(mode=0o700,parents=False,exist_ok=False)
 sock,stream=q.qga_open(str(a.socket),timeout=30)
 def call(name,args):
  q.send(stream,{'execute':name,'arguments':args});r=q.recv(stream)
  if 'error' in r:raise RuntimeError(r['error'])
  return r['return']
 def read(name,limit):
  h=call('guest-file-open',{'path':'C:/ProgramData/KayfabeDumpM/'+name,'mode':'rb'})
  data=bytearray()
  try:
   for _ in range(limit//CHUNK+2):
    r=call('guest-file-read',{'handle':h,'count':min(CHUNK,limit-len(data)+1)})
    block=base64.b64decode(r.get('buf-b64',''),validate=True)
    if len(block)!=r['count'] or len(data)+len(block)>limit:raise ValueError('Export read exceeds bound/count mismatch')
    data.extend(block)
    if r['eof']:return bytes(data)
    if not block:raise ValueError('Read made no progress')
   raise ValueError('Read chunk bound exceeded')
  finally:call('guest-file-close',{'handle':h})
 try:
  if a.action=='stage':
   for name,data in blobs.items():
    h=call('guest-file-open',{'path':'C:/ProgramData/KayfabeDumpMTools/'+name,'mode':'wb'})
    try:
     for offset in range(0,len(data),CHUNK):
      block=data[offset:offset+CHUNK]
      r=call('guest-file-write',{'handle':h,'buf-b64':base64.b64encode(block).decode()})
      if r['count']!=len(block):raise ValueError('Short write')
     call('guest-file-flush',{'handle':h})
    finally:call('guest-file-close',{'handle':h})
   print('Pinned wrapper and bundle staged; Windows recipe rechecks both hashes')
  else:
   raw=read('export.json',64*1024)
   manifest=json.loads(raw.decode('utf-8-sig'))
   if manifest.get('schema')!='kayfabe-private-dump-export/1':raise ValueError('Unknown export schema')
   rows=manifest['files'];seen=set()
   if not isinstance(rows,list) or len(rows)>len(NAMES):raise ValueError('Export file count')
   for r in rows:
    name=r['name'];size=r['bytes'];sha=r['sha256']
    if name not in NAMES or name in seen or type(size) is not int or not 0<=size<=LIMIT:raise ValueError('Unrecognized export entry')
    seen.add(name)
    data=read(name,LIMIT)
    if len(data)!=size or hashlib.sha256(data).hexdigest()!=sha:raise ValueError('Export length/hash mismatch: '+name)
    with (a.directory/name).open('xb') as f:f.write(data)
   with (a.directory/'export.json').open('xb') as f:f.write(raw)
   print(json.dumps({'files':len(seen),'verified':True,'private_directory':str(a.directory)}))
 finally:sock.close()

if __name__=='__main__':main()
