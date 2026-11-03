"""Bounded rental-only observation times for public RPC metadata (not driver-generation timestamps)."""
import sys,time,datetime,json,re,os,signal
from pathlib import Path
log,out,pid=Path(sys.argv[1]),Path(sys.argv[2]),int(sys.argv[3]);assert log.is_file() and not out.exists();stop=False

def stopped(*_):
 global stop;stop=True
signal.signal(signal.SIGTERM,stopped)
pat=re.compile(r'kf-rm: rpc-trace fn=(\d+) ([A-Za-z0-9_]+(?:\(\d+\))?) seq=(\d+) ?(?:cmd=(0x[0-9a-f]+)|class=(0x[0-9a-f]+))?.*? result=(\S+)')
start=time.monotonic();rows=bad=0;reason='duration_limit';source_bytes=0;identity=log.stat().st_ino;pending=b''
with log.open('rb') as src,out.open('w') as sink:
 while time.monotonic()-start<900 and rows<32768 and source_bytes<64*1024*1024:
  raw=src.readline(128*1024+1)
  if raw:
   source_bytes+=len(raw)
   if len(raw)>128*1024:reason='line_limit';break
   pending+=raw
   if len(pending)>128*1024:reason='line_limit';break
   if not pending.endswith(b'\n'):time.sleep(.1);continue
   line=pending.decode(errors='replace');pending=b'';m=pat.search(line)
   if 'kf-rm: rpc-trace' in line and not m:bad+=1
   if m:
    r={'ordinal':rows,'observed_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'observer_monotonic_ns':time.monotonic_ns(),'function':int(m[1]),'kind':m[2],'sequence':int(m[3]),'selector':m[4] or m[5] or '-','trace_result':m[6],'file_offset_after':source_bytes};sink.write(json.dumps(r,separators=(',',':'))+'\n');sink.flush();rows+=1
   continue
  if log.stat().st_ino!=identity:reason='log_replaced';break
  if stop:reason='stop_after_eof';break
  if not Path(f'/proc/{pid}/exe').exists():reason='qemu_exited_at_eof';break
  time.sleep(.1)
receipt={'rows':rows,'unparsed_rpc_lines':bad,'reason':reason,'source_bytes_read':source_bytes,'source_size_at_end':log.stat().st_size,'complete_prefix':True,'incomplete_line_bytes':len(pending),'reached_eof':src.closed,'observation_time_not_driver_generation_time':True,'limits':{'seconds':900,'records':32768,'source_bytes':64*1024*1024,'line_bytes':128*1024}}
receipt['reached_eof']=source_bytes==log.stat().st_size
out.with_suffix('.receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
