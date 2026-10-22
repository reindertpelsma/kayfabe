import datetime as dt,json,re,time
from pathlib import Path

def run(agent,R,S,A,boot):
 root=r'C:\Users\Public\codex-Q4-smokes';rows={};runid=A.name.rsplit('-',1)[1]
 def query(ps,name):return json.loads(agent(ps,name))
 def health(name):
  ps="$b=[DateTime]::Parse('"+boot+"');$e=@(Get-WinEvent -FilterHashtable @{LogName='System';StartTime=$b;ProviderName=@('nvlddmkm','Display')} -ErrorAction SilentlyContinue | Where-Object {$_.Id -in @(153,4101)} | ForEach-Object {@{id=$_.Id;time=$_.TimeCreated.ToString('o')}});@{events=$e;boot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToString('o')} | ConvertTo-Json -Depth 5 -Compress"
  h=query(ps,name);assert not (A/'arm-stop.json').exists(),'HOST_SAFETY_STOP';return h
 agent((R/'install-q4-smokes.ps1').read_text(),'smoke-install.txt')
 inventory=query((R/'app-paths.ps1').read_text(),'app-paths.json');rows['inventory']=inventory
 launch="$ErrorActionPreference='Stop';if (@(Get-Process Heaven -ErrorAction SilentlyContinue).Count -ne 0){throw 'PREEXISTING_HEAVEN_REFUSED'};& C:\\kf\\ogl\\runi.ps1 -Name Q4-heaven-"+runid+" -Cmd 'powershell -NoProfile -ExecutionPolicy Bypass -File "+root+"\\heaven.ps1 -Api direct3d11 -Fs 0 -Tag Q4-heaven-"+runid+"'"
 agent(launch,'heaven-start.txt')
 start=time.monotonic();last=None
 for n in range(65):
  ps="$p='C:\\kf\\apps\\out\\Q4-heaven-"+runid+"';$j=@{};foreach($x in @('.txt','.stdout.txt','.stderr.txt')){if(Test-Path ($p+$x)){$j[$x]=Get-Content ($p+$x) -Raw}};$j | ConvertTo-Json -Compress"
  last=query(ps,f'heaven-status-{n:03}.json');h=health(f'heaven-health-{n:03}.json')
  if h['events']:break
  if re.search(r'END rc=',last.get('.txt','')):break
  time.sleep(15)
 text='\n'.join(last.values());rows['heaven']={'status':'PASS' if 'END rc=0' in text and re.search(r'FPS|Score',text,re.I) and not h['events'] else 'FAIL','duration_seconds':time.monotonic()-start,'output':last,'health':h,'settings':'1920x1080 windowed D3D11 Ultra; tessellation normal; AA 0'}
 # The proven launcher reads its saved command only inside the guest; never export it.
 mc_exists=all(inventory['minecraft_paths'].get(p,False) for p in [r'C:\kf\ogl\mc.ps1',r'C:\kf\ogl\mc_cmd.txt'])
 if mc_exists:
  agent("& C:\\kf\\ogl\\runi.ps1 -Name Q4-minecraft-"+runid+" -Cmd 'powershell -NoProfile -ExecutionPolicy Bypass -File "+root+"\\minecraft-wrapper.ps1'",'minecraft-start.txt')
  states=[];t0=time.monotonic();live_since=None;live_seconds=0
  for n in range(50):
   ps="$r='"+root+"';$j=@{processes=@(Get-CimInstance Win32_Process -Filter \"Name='javaw.exe'\" | Select-Object ProcessId,ParentProcessId,CreationDate)};foreach($f in @('mc-owned.json','mc-exit.json')){if(Test-Path \"$r\\$f\"){$j[$f]=Get-Content \"$r\\$f\" -Raw}};$j|ConvertTo-Json -Depth 5 -Compress"
   state=query(ps,f'minecraft-status-{n:03}.json');states.append(state);h=health(f'minecraft-health-{n:03}.json')
   if 'mc-exit.json' in state or h['events']:break
   if 'mc-owned.json' in state and state['processes']:
    if live_since is None:live_since=time.monotonic()
    live_seconds=time.monotonic()-live_since
   if live_seconds>=600:break
   time.sleep(15)
  agent("'' | Set-Content '"+root+"\\mc-stop.flag'",'minecraft-stop-request.txt')
  time.sleep(15);final=query(ps,'minecraft-final.json')
  alive_seconds=live_seconds;rows['minecraft']={'status':'PASS' if alive_seconds>=600 and not h['events'] and 'mc-owned.json' in states[-1] and 'mc-exit.json' not in states[-1] else 'FAIL','elapsed_seconds':alive_seconds,'samples':len(states),'final':final,'health':h,'scope':'Process/guest health smoke; world/render observation recorded separately'}
 else:rows['minecraft']={'status':'NOT_INSTALLED_OR_NO_HARNESS'}
 rows['steel_nomad']={'status':'AWAITING_UI_RUN' if inventory.get('three_dmark_dirs') else 'NOT_INSTALLED','installed_dirs':inventory.get('three_dmark_dirs',[])}
 rows['app_matrix']={'status':'NOT_PRESENT' if not inventory.get('matrix_scripts') else 'AWAITING_INSTALLED_HARNESS','scripts':inventory.get('matrix_scripts',[])}
 (S/'smokes-ready-for-manual.json').write_text(json.dumps(rows,indent=2)+'\n')
 if inventory.get('three_dmark_dirs'):
  deadline=min(time.monotonic()+1800,time.monotonic()+max(0,dt.datetime.fromisoformat('2026-10-22T17:10:00+00:00').timestamp()-time.time()))
  while not (S/'manual-smokes-complete.json').exists() and time.monotonic()<deadline:
   health('manual-smokes-health.json');time.sleep(15)
  if (S/'manual-smokes-complete.json').exists():rows.update(json.loads((S/'manual-smokes-complete.json').read_text()))
  else:rows['steel_nomad']['status']='UNVERIFIED_UI_RUN_NOT_COMPLETED'
 rows['health_final']=health('regression-health-final.json')
 rows['verdict']='FAIL' if any(isinstance(v,dict) and v.get('status')=='FAIL' for v in rows.values()) or rows['health_final']['events'] else 'PARTIAL'
 (S/'regressions.json').write_text(json.dumps(rows,indent=2)+'\n');return rows
