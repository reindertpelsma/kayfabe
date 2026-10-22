# Reused from our public e34886891 Blackwell run kit; no decoder changes.
"""Rental-only bounded projection of posted public RPC allocation headers; bodies stay rental."""
import json,re,hashlib,struct,sys
from pathlib import Path
p=Path(sys.argv[1]);out=Path(sys.argv[2]);assert p.is_file() and p.stat().st_size<=64*1024*1024
rows=[];whole=hashlib.sha256();source_bytes=0
pattern=re.compile(r'kf-gsp: reply-body record=(\d+) message_seq=(\d+) function=(0x[0-9a-f]+) selector=(\S+) rpc_result=(0x[0-9a-f]+) rpc_result_private=(0x[0-9a-f]+) total=(\d+) saved=(\d+) truncated=(\d+) body=([0-9a-f]*)')
with p.open('rb') as f:
 for raw in f:
  whole.update(raw);source_bytes+=len(raw)
  if len(raw)>128*1024:continue
  m=pattern.search(raw.decode(errors='replace'))
  if not m or int(m[3],16)!=103:continue
  if len(rows)>=2048: break
  body=bytes.fromhex(m[10]);saved=int(m[8]);assert len(body)==saved and saved<=4096
  row={'diagnostic_record':int(m[1]),'posted_message_sequence':int(m[2]),'function':m[3],'class_selector':m[4],'envelope_result':m[5],'envelope_private_result':m[6],'posted_payload_bytes':int(m[7]),'saved_bytes_on_rental':saved,'body_truncated_on_rental':bool(int(m[9])),'header_decode':'TRUNCATED','private_body_exported':False}
  # Public rpc_gsp_rm_alloc_v03_00 (OGKM580pin307159f...), shared with
  # kf-abi::DriverAbiTable::decode_rpc_alloc: class@12,status@16,paramsSize@20.
  # Read none of hClient/hParent/hObject or any trailing params.
  if saved>=32:
   cls,status,size=struct.unpack_from('<III',body,12);row.update(header_decode='PUBLIC_FIXED_HEADER',class_from_header=hex(cls),body_status=hex(status),declared_params_bytes=size)
  rows.append(row)
out.write_text(''.join(json.dumps(x,sort_keys=True)+'\n' for x in rows))
receipt={'source_bytes':source_bytes,'source_sha256':whole.hexdigest(),'posted_allocation_records':len(rows),'allocation_header_layout':'public rpc_gsp_rm_alloc_v03_00, min32,class12,status16,paramsSize20','raw_payloads_remain_rental':True,'diagnostic_limit':'512records/256KiB across selected replies; no full transport/event completeness claim','export_sha256':hashlib.sha256(out.read_bytes()).hexdigest()}
out.with_suffix('.receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt))
