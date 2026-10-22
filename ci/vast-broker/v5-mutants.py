#!/usr/bin/env python3
"""V5 behavioural mutations; run sequentially, never beside baseline tests."""
import argparse,ast,json,re,subprocess
from pathlib import Path
BASE=Path(__file__).resolve().parent
CASES=[
 ('default-price','defaults.json',[(r'"price": 1',r'"price": 0.5')],['v5-lifecycle.test.js','v5-price.test.js']),
 ('default-daily','defaults.json',[(r'"daily": 6',r'"daily": 3')],['v5-lifecycle.test.js']),
 ('default-reserve','defaults.json',[(r'"reserve": 8',r'"reserve": 5')],['v5-lifecycle.test.js']),
 ('default-rolling','defaults.json',[(r'"rolling": 30',r'"rolling": 60')],['v5-lifecycle.test.js']),
 ('default-boot','defaults.json',[(r'"bootMinutes": 40',r'"bootMinutes": 10')],['v5-lifecycle.test.js']),
 ('default-hard','defaults.json',[(r'"hardMinutes": 180',r'"hardMinutes": 20')],['v5-lifecycle.test.js']),
 ('default-soft-margin','defaults.json',[(r'"softMarginMinutes": 10',r'"softMarginMinutes": 1')],['v5-lifecycle.test.js']),
 ('default-manual-ceiling','defaults.json',[(r'"manualCeilingMinutes": 480',r'"manualCeilingMinutes": 60')],['v5-lifecycle.test.js']),
 ('default-pass-grace','defaults.json',[(r'"passGraceMinutes": 5',r'"passGraceMinutes": 10')],['v5-lifecycle.test.js']),
 ('default-fail-grace','defaults.json',[(r'"failGraceMinutes": 10',r'"failGraceMinutes": 5')],['v5-lifecycle.test.js']),
 ('create-reservation-phase','src/index.js',[("phase:'CREATING',bootDeadline:","phase:'BOOTING',bootDeadline:")],['v5-lifecycle.test.js']),
 ('boot-phase','src/index.js',[("r.phase='BOOTING';await ledger.save(r);","r.phase='CREATING';await ledger.save(r);")],['v5-lifecycle.test.js']),
 ('immutable-clock','src/index.js',[("if(r.hardDeadline===undefined)r.hardDeadline=r.started+(r.ttl??cfg.hard);","r.hardDeadline=now()+(r.ttl??cfg.hard);")],['v5-lifecycle.test.js']),
 ('boot-timeout','src/index.js',[("record.phase!=='MANUAL'&&record.firstReportAt===undefined&&now()>=record.bootDeadline","false")],['v5-lifecycle.test.js']),
 ('heartbeat-timeout','src/index.js',[("record.phase!=='MANUAL'&&record.lastPostAt!==null&&now()-record.lastPostAt>=record.heartbeatTimeout","false")],['v5-lifecycle.test.js']),
 ('empty-heartbeat','src/index.js',[("r.lastPostAt=now();","r.lastPostAt??=now();")],['v5-lifecycle.test.js']),
 ('hard-in-manual','src/index.js',[("record.hardDeadline!==null&&now()>=record.hardDeadline","false")],['v5-lifecycle.test.js']),
 ('manual-ceiling','src/index.js',[("record.phase==='MANUAL'&&now()>=record.manualDeadline","false")],['v5-lifecycle.test.js']),
 ('hold-grace-and-heartbeat','src/index.js',[("r.phase='MANUAL';r.manualAt??=now();r.resultDeadline=null;","r.manualAt??=now();")],['v5-lifecycle.test.js']),
 ('hold-auth','src/index.js',[("if(!r||!await tokens.verify(bearer,r)||r.ended!==null||r.closed||now()>=r.reportExpiry*1000)","if(!r||r.ended!==null||r.closed||now()>=r.reportExpiry*1000)")],['v5-lifecycle.test.js']),
 ('owner-timer-auth','src/index.js',[("if(match?.[1]==='stop-timer'){if(!owner)refuse('UNAUTHORIZED',401);","if(match?.[1]==='stop-timer'){")],['v5-lifecycle.test.js']),
 ('timer-extension-liability','src/index.js',[("if(typeof credit!=='number'||!Number.isFinite(credit)||credit<cfg.reserve+remaining)refuse('UNAVAILABLE_BUDGET',409);","/* mutant omits credit */"),("await ledger.budgets(cfg,now(),total,r.key);","/* mutant omits extended budget */")],['v5-lifecycle.test.js']),
 ('manual-cron-status','src/index.js',[("if(record.phase==='MANUAL')await workerLine(record,'MANUAL hard_timer='+(record.hardDeadline===null?'removed':'active')+' ceiling_at='+record.manualDeadline,cfg);","/* mutant drops MANUAL cron line */")],['v5-lifecycle.test.js','v5-manual-errors.test.js']),
 ('worker-tail-bounds','src/index.js',[("appendWorkerText(r,text);await ledger.save(r);","r.log=((r.log??'')+text).slice(-8192);await ledger.save(r);")],['v5-worker-tail.test.js']),
 ('creation-based-soft-deadline','src/index.js',[("runnerNonce:r.runnerNonce,softDeadline:r.softDeadline","runnerNonce:r.runnerNonce")],['v5-lifecycle.test.js']),

 ('hold-helper-scope','src/onstart.js',[("/hold/${callback.run}'","/hold/999-1'")],['v5-onstart.test.js']),
 ('ssh-login-hold','src/onstart.js',[(r'if [ -n "\${SSH_CONNECTION:-}" ]; then',r'if false; then')],['v5-onstart.test.js']),
 ('heartbeat-after-cap','src/onstart.js',[("if kind!='result' and not heartbeat_post and","if kind!='result' and")],['v5-onstart.test.js']),
 ('heartbeat-busy-branch','src/onstart.js',[("        if current-heartbeat>=","        elif current-heartbeat>=")],['v5-onstart.test.js']),
 ('heartbeat-ten-minutes','defaults.json',[(r'"heartbeatMinutes": 10',r'"heartbeatMinutes": 20')],['v5-onstart.test.js','v5-lifecycle.test.js']),
 ('soft-expired-no-work','src/onstart.js',[("if remaining>0 else 'exit 124", "if remaining>0 else 'exit 0")],['v5-soft-deadline.test.js']),
 ('shipped-numeric-overrides','wrangler.toml',[(r'RENTING_ENABLED = "false"', 'RENTING_ENABLED = "false"\nMAX_PRICE_USD_HOUR = "0.5"\nTTL_MINUTES = "20"')],['v5-config.test.js']),
]
def main():
    ap=argparse.ArgumentParser();ap.add_argument('--filter',default='');ap.add_argument('--results',type=Path);ap.add_argument('--preflight',action='store_true');args=ap.parse_args();results=[]
    for name,file,patches,tests in CASES:
        if args.filter and args.filter not in name:continue
        path=BASE/file;original=path.read_text();changed=original
        for old,new in patches:
            assert changed.count(old)==1,(name,old);changed=changed.replace(old,new)
        if args.preflight:continue
        try:
            path.write_text(changed)
            if path.suffix=='.json':json.loads(changed)
            elif path.suffix=='.py':ast.parse(changed)
            elif path.suffix=='.toml':
                import tomllib
                tomllib.loads(changed)
            else:
                syntax=subprocess.run(['node','--check',str(path)],capture_output=True,text=True);assert syntax.returncode==0,(name,syntax.stderr)
            run=subprocess.run(['node','--test',*[str(BASE/'test'/t) for t in tests]],capture_output=True,text=True,timeout=90)
            killers=re.findall(r'^not ok \d+ - (.+)$',run.stdout,re.M);assert run.returncode and len(killers)>=2,(name,killers,run.stdout[-2000:])
            results.append({'mutant':name,'count':len(killers),'killers':killers});print(name+': '+str(len(killers))+' killers',flush=True)
            if args.results:args.results.write_text(json.dumps(results,indent=2)+'\n')
        finally:path.write_text(original)
    if args.preflight:print('V5 anchors valid:',len(CASES))
if __name__=='__main__':main()
