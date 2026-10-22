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
 ('mandatory-verification','src/policy.js',[("if(!(offer.verified===true||offer.verified===undefined&&offer.verification==='verified'))", "if(false)")],['v5-offers.test.js']),
 ('host-capability','src/policy.js',[("if(!Number.isSafeInteger(offer.id)||offer.id<=0||offer.vms_enabled!==true||offer.rentable!==true||offer.rented!==false||offer.num_gpus!==1||!enough('disk_space',DEFAULTS.diskGb))", "if(false)")],['v5-offers.test.js']),
 ('known-bad-machine','src/policy.js',[("cfg.badMachines.includes(offer.machine_id)","false")],['v5-offers.test.js']),
 ('quality-reliability','src/policy.js',[("if(!enough('reliability',q.reliability)||offer.reliability>1)","if(false)")],['v5-offers.test.js']),
 ('quality-cpu-ram','src/policy.js',[("if(!enough('cpu_cores_effective',q.cores)||!enough('cpu_ghz',q.cpuGhz)||!enough('cpu_ram',q.ramMb))", "if(false)")],['v5-offers.test.js']),
 ('quality-relative-performance','src/policy.js',[("offer.dlperf<median*q.medianRatio", "false")],['v5-offers.test.js']),
 ('quality-network','src/policy.js',[("if(!enough('inet_down',q.downMbps)||!enough('inet_up',q.upMbps))", "if(false)")],['v5-offers.test.js']),
 ('quality-disk','src/policy.js',[("if(!enough('disk_bw',q.diskMbps))", "if(false)")],['v5-offers.test.js']),
 ('quality-driver','src/policy.js',[("if(!cfg.driverTags.includes(offer.driver_version))", "if(false)")],['v5-offers.test.js']),
 ('turing-class-floor','src/policy.js',[("offer.compute_cap<750", "false")],['v5-offers.test.js']),
 ('gpu-deny-list','src/policy.js',[("if(cfg.gpuDeny.some(match))return null;", "/* mutant ignores GPU deny-list */")],['v5-offers.test.js']),
 ('effective-price-includes-network','src/policy.js',[("const effective=computeStorage+bandwidth/(cfg.hard/3600000);", "const effective=computeStorage;") ,("profile==='check'?cfg.price:cfg.sweepPrice)*(cfg.hard+cfg.cronPeriod)/3600000)", "profile==='check'?cfg.price:cfg.sweepPrice)*1000)")],['v5-offers.test.js']),
 ('storage-consistency','src/policy.js',[("if(Math.abs(offer.dph_total-computeStorage)>Math.max(1e-7,computeStorage*1e-6))return null;", "/* mutant trusts inconsistent allocated storage */")],['v5-offers.test.js']),
 ('check-die-enforcement','src/policy.js',[("if(!die||!(profile==='check'?cfg.checkDies:cfg.sweepDies).includes(die))", "if(!die)")],['v5-offers.test.js']),
 ('sweep-owner-only','src/index.js',[("if(profile==='sweep'&&(c.actor_id!==cfg.owner||c.event_name!=='workflow_dispatch'||c.workflow_ref!==cfg.repo+'/.github/workflows/gpu-vast-sweep.yml@refs/heads/master'))refuse('UNAUTHORIZED',401);", "/* mutant omits profile owner gate */")],['v5-offers.test.js']),
 ('preference-order-forced','src/offers.js',[("order=preferences.filter(x=>set.includes(x));", "order=set;")],['v5-offers.test.js']),
 ('rotation-ledger','src/offers.js',[("previous=await ledger.get('rotation:lastDie')", "previous=undefined")],['v5-offers.test.js']),
 ('rotation-skip-recent','src/offers.js',[("if(cfg.skipPassedDays>0)", "if(false)")],['v5-offers.test.js']),
 ('offers-sixty-second-cache','src/offers.js',[("let hit=this.at!==null&&this.now()-this.at<DEFAULTS.availabilityCacheMs;", "let hit=false;")],['v5-offers.test.js']),
 ('admission-rate-bound','src/index.js',[("if(++admissionCount>DEFAULTS.admissionPerMinute)", "if(false)")],['v5-offers.test.js']),
 ('separate-profile-daily','src/ledger.js',[("(v.profile??'check')===(cfg.profile??'check')", "true")],['v5-offers.test.js']),
 ('network-liability-not-amortised','src/ledger.js',[("price:record.computeStoragePrice??record.price,bandwidthCost:record.bandwidthCost??0", "price:record.price,bandwidthCost:0"), ("price:record.computeStoragePrice??record.price,bandwidthCost:record.bandwidthCost??0", "price:record.price,bandwidthCost:0")],['v5-network-liability.test.js']),
 ('learn-frozen-kvm','src/index.js',[("record.bootFailure?.confirmed", "false")],['v5-machine.test.js']),
 ('force-choice-on-refusal','src/index.js',[("e.selection={die:offer.die,ignored:chosen.ignored,reason:chosen.reason}", "e.selection=undefined")],['v5-refusal-selection.test.js']),

]
def main():
    ap=argparse.ArgumentParser();ap.add_argument('--filter',default='');ap.add_argument('--results',type=Path);ap.add_argument('--preflight',action='store_true');args=ap.parse_args();results=[]
    for name,file,patches,tests in CASES:
        if args.filter and args.filter not in name:continue
        path=BASE/file;original=path.read_text();changed=original
        for old,new in patches:
            assert old in changed,(name,old);changed=changed.replace(old,new,1)
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
