from pathlib import Path
import tempfile,xml.sax.saxutils as sx,json,contextlib,io
source=Path(__file__).with_name('project.py').read_text()
ns='http://schemas.microsoft.com/win/2004/08/events/event'
def event(i,provider,eid,utc,data):
 fields=''.join('<Data Name="'+sx.escape(k)+'">'+sx.escape(v)+'</Data>' for k,v in data.items())
 return f'<Event xmlns="{ns}"><System><Provider Name="{provider}"/><EventID>{eid}</EventID><EventRecordID>{i}</EventRecordID><TimeCreated SystemTime="{utc}"/></System><EventData>{fields}</EventData></Event>'
now='2026-10-19T22:27:00.0000000Z';old='2026-10-19T21:27:00.0000000Z'
rows=[event(1,'Display',4101,now,{}),event(2,'Display',4101,old,{}),event(3,'OtherProvider',4101,now,{}),event(4,'Microsoft-Windows-WER-SystemErrorReporting',1001,now,{'param1':'0x00000116 (0xffffdeadbeef1234)'}),event(5,'Microsoft-Windows-WER-SystemErrorReporting',1001,now,{'param1':'0x00000119 (0xffffdeadbeef1234)'}),event(6,'nvlddmkm',153,now,{'private':'DO_NOT_EXPORT_SECRET_SENTINEL'})]
pci='PCI\\VEN_10DE&DEV_2B85&SUBSYS_DUMMY'
pnp=[event(7,'Microsoft-Windows-Kernel-PnP',411,now,{'DeviceInstanceId':pci,'Problem':'0x2b','DriverName':'oem7.inf'}),event(8,'Microsoft-Windows-Kernel-PnP',411,old,{'DeviceInstanceId':pci,'Problem':'0x2b'}),event(9,'Microsoft-Windows-Kernel-PnP',411,now,{'DeviceInstanceId':'PCI\\VEN_8086&DEV_1234','Problem':'0x2b'}),event(10,'Microsoft-Windows-Kernel-PnP',411,now,{'DeviceInstanceId':pci,'Problem':'0'})]
with tempfile.TemporaryDirectory(prefix='sweep-evtx-') as tmp:
 root=Path(tmp)
 for name,items in [('System',rows),('Microsoft-Windows-Kernel-PnP%4Configuration',pnp)]:
  (root/(name+'.evtx')).write_bytes(b'FIXTURE_ONLY')
  (root/(name+'.evtx.xml')).write_text('\n'.join(items))
 out=root/'projection.json'
 code=source.replace("Path('/root/prov/bw580_on_evtx_raw')",'Path('+repr(str(root))+')').replace("Path('/root/prov/bw580_on_evtx_public.json')",'Path('+repr(str(out))+')')
 with contextlib.redirect_stdout(io.StringIO()):exec(compile(code,'fixture-projection','exec'),{})
 j=json.loads(out.read_text())
 assert [r['record_id'] for r in j['TDR_positive_records']]==[1,4]
 assert [r['record_id'] for r in j['PNP_Code43_positive_records']]==[7]
 assert {r['record_id'] for r in j['selected_events']}=={1,2,4,5,6,7,8,10}
 assert sum(r['records_parsed'] for r in j['logs'])==10
 assert '0xffffdeadbeef1234' not in out.read_text()
 assert 'DO_NOT_EXPORT_SECRET_SENTINEL' not in out.read_text()
 print('EVTX_PROJECTION_FIXTURE_PASS records=10 positives_and_negative_guards_checked=1 private_field_redaction=1')
