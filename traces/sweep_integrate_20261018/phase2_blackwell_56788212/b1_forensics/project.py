"""Rental-only bounded Windows EVTX projection; no messages, paths or raw payload exported."""
from pathlib import Path
import json,re,hashlib,xml.etree.ElementTree as ET,datetime
from event_time import event_time
root=Path('/root/prov/bw580_on_evtx_raw')
ns={'e':'http://schemas.microsoft.com/win/2004/08/events/event'}
allowed={'Display','nvlddmkm','Microsoft-Windows-Kernel-PnP','Microsoft-Windows-WER-SystemErrorReporting'}
start=datetime.datetime.fromisoformat('2026-10-19T22:22:02+00:00')
end=datetime.datetime.fromisoformat('2026-10-19T22:32:03+00:00')
selected=[];receipts=[]
for p in sorted(root.glob('*.evtx.xml')):
 assert p.stat().st_size<=128*1024*1024
 raw=p.read_bytes();assert b'<!DOCTYPE' not in raw and b'<!ENTITY' not in raw
 blocks=re.findall(rb'<Event(?:\s[^>]*)?>.*?</Event>',raw,re.S)
 assert blocks and len(blocks)<=20000 and len(blocks)==raw.count(b'</Event>'),'No complete bounded Event XML sequence'
 parsed=0
 for b in blocks:
  assert len(b)<=512*1024
  x=ET.fromstring(b);sys=x.find('e:System',ns)
  assert sys is not None
  provider=sys.find('e:Provider',ns).get('Name','')
  event_id=int(sys.findtext('e:EventID','-1',ns))
  utc=sys.find('e:TimeCreated',ns).get('SystemTime','')
  # Preserve the full original timestamp; only window comparison uses microsecond precision.
  stamp=event_time(utc)
  parsed+=1
  if provider not in allowed:continue
  data={z.get('Name',''):z.text or '' for z in x.findall('e:EventData/e:Data',ns)}
  pci=any('VEN_10DE&DEV_2B85' in v.upper() for k,v in data.items() if 'device' in k.lower())
  if provider=='Microsoft-Windows-Kernel-PnP' and not pci:continue
  fields={}
  for key in ('Problem','Status','ProblemStatus','ConfigFlags'):
   value=data.get(key,'')
   if re.fullmatch(r'0x[0-9a-fA-F]{1,8}|[0-9]{1,10}',value):
    n=int(value,16 if value.startswith('0x') else 10)
    if n<=0xffffffff:fields[key]=hex(n)
  for key in ('ServiceName','DriverName'):
   value=data.get(key,'')
   if value=='nvlddmkm' or re.fullmatch(r'oem[0-9]{1,4}\.inf',value,re.I):fields[key]=value
  if pci:fields['public_PCI']='10de:2b85'
  bugcheck=[]
  if provider=='Microsoft-Windows-WER-SystemErrorReporting':
   bugcheck=sorted(set(m.lower() for v in data.values() for m in re.findall(r'\b0x00000(?:116|117)\b',v,re.I)))
  selected.append({'log':p.name.removesuffix('.evtx.xml'),'record_id':int(sys.findtext('e:EventRecordID','0',ns)),'utc':utc,'provider':provider,'event_id':event_id,'within_ON_boot_to_cleanup':start<=stamp<=end,'fields':fields,'video_bugcheck_codes':bugcheck,'opaque_message_and_other_fields_omitted':True})
 receipts.append({'log':p.name,'XML_bytes':len(raw),'XML_sha256':hashlib.sha256(raw).hexdigest(),'records_parsed':parsed,'EVTX_bytes':p.with_suffix('').stat().st_size,'EVTX_sha256':hashlib.sha256(p.with_suffix('').read_bytes()).hexdigest()})
assert len(receipts)==2
within=[r for r in selected if r['within_ON_boot_to_cleanup']]
tdr=[r for r in within if (r['provider']=='Display' and r['event_id']==4101) or r['video_bugcheck_codes']]
pnp43=[r for r in within if r['provider']=='Microsoft-Windows-Kernel-PnP' and r['event_id']==411 and r['fields'].get('Problem')=='0x2b']
out={'classification':'MEASURED persisted public Windows event fields only','boot_window_UTC':[start.isoformat(),end.isoformat()],'read_only_image':'bw580_on.qcow2 stopped; nbdRO=1, no guest boot or driver replay','logs':receipts,'selected_events':selected,'within_window_events':len(within),'TDR_positive_records':tdr,'PNP_Code43_positive_records':pnp43,'absence_limit':'Absence in persisted logs after QMP quit does not prove no TDR/Code43; unflushed records may be lost. No raw EVTX/XML/messages exported.'}
Path('/root/prov/bw580_on_evtx_public.json').write_text(json.dumps(out,indent=2)+'\n')
print(json.dumps({'parsed':sum(r['records_parsed'] for r in receipts),'selected':len(selected),'within_window':len(within),'TDR_positive':len(tdr),'Code43_positive':len(pnp43)}))
