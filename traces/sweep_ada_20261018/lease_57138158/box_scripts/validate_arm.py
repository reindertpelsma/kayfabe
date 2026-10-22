from pathlib import Path
import sys,re,json
run=sys.argv[1];root=Path('/root/prov');log=(root/('arm_'+run+'.log')).read_text()
held=re.findall(r'ARM_HOLD_OBSERVED seconds=([0-9]+)',log);assert len(held)==1 and int(held[0])>=600
for suffix in ('health_before','health_after'):
 text=(root/('arm_'+run+'_'+suffix+'.txt')).read_text();assert 'S2_HEALTH_PASS ' in text
 values=[json.loads(l.split(' ',1)[1]) for l in text.splitlines() if l.startswith('S2_HEALTH ')];assert len(values)==1
 if suffix=='health_before':before=values[0]
 else:assert values[0]['boot']==before['boot']
text=(root/('arm_'+run+'_accept.txt')).read_text()
for needle in ('WIN_ACCEPT_EXIT rc=0 ','CE rv=0xabcd1234 want=0xabcd1234 -> PASS','KERNEL rv=43 want=43 -> PASS','CUP8 RESULT N=2048 bad=0 maxerr=0 '):assert needle in text,needle
assert text.count('KFDX RESULT OK')==2
assert 'ARM_EXIT run='+run+' rc=0 ' in log
print('S2_WINDOWS_ARM_PASS',run,'hold_seconds='+held[0],'zero4101_153_bugcheck=1')
