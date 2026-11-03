"""Rental-local public Microsoft PDB proxy. A reverse SSH port reaches it from the isolated collector VM."""
from http.server import BaseHTTPRequestHandler,HTTPServer
import urllib.request,urllib.error,re,hashlib
from pathlib import Path
class Handler(BaseHTTPRequestHandler):
 served_bytes=0
 allowed={'dxgkrnl.pdb','dxgmms2.pdb','ntkrnlmp.pdb','ntoskrnl.pdb','win32kbase.pdb','win32kfull.pdb'}
 def do_GET(self):
  if len(self.path)>512 or not re.fullmatch(r'/[A-Za-z0-9._/-]+',self.path) or '..' in self.path.split('/'):
   self.send_error(400);return
  pieces=self.path.strip('/').split('/')
  if len(pieces)!=3 or pieces[0].lower() not in self.allowed or not re.fullmatch('[A-Fa-f0-9]{8,64}',pieces[1]) or pieces[2] not in [pieces[0],pieces[0][:-1]+'_','file.ptr']:
   self.send_error(403);return
  cache=Path('/root/prov/public_symbols')/hashlib.sha256(self.path.encode()).hexdigest();cache.parent.mkdir(exist_ok=True)
  try:
   if not cache.exists():
    req=urllib.request.Request('https://msdl.microsoft.com/download/symbols'+self.path,headers={'User-Agent':'Microsoft-Symbol-Server/10.0'})
    with urllib.request.urlopen(req,timeout=60) as src:
     body=src.read(64*1024*1024+1)
    if len(body)>64*1024*1024 or sum(p.stat().st_size for p in cache.parent.iterdir())+len(body)>128*1024*1024:self.send_error(413);return
    cache.write_bytes(body)
   body=cache.read_bytes()
   if Handler.served_bytes+len(body)>128*1024*1024:self.send_error(413);return
   Handler.served_bytes+=len(body);self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  except urllib.error.HTTPError as e:self.send_error(e.code)
  except Exception:self.send_error(502)
 def log_message(self,fmt,*args):print(fmt%args,flush=True)
server=HTTPServer(('127.0.0.1',18766),Handler)
server.timeout=120
for request in range(256):server.handle_request()
server.server_close()
