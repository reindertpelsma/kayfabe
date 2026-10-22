#!/usr/bin/env python3
"""Current round 3 mutations; each must kill >=2 independently named tests."""
import argparse,json,re,subprocess
from pathlib import Path
BASE=Path(__file__).resolve().parent
CASES=[
 ('runner-scrub','runner.py',[("    original=len(raw);text=raw.decode('utf-8',errors='replace')","    return raw.decode('utf-8',errors='replace'),0\n    original=len(raw);text=raw.decode('utf-8',errors='replace')")],'round3-runner.test.js'),
 ('runner-schema','runner.py',[("if key in result:raise Invalid()","if False:raise Invalid()"),("if not set(data)<=STATUS_KEYS or",'if False or'),("if set(data)!=STATUS_KEYS:raise Invalid()","if False:raise Invalid()")],'round3-runner.test.js'),
 ('runner-binding','runner.py',[("if data['run']!=ctx['run'] or data['sha']!=ctx['sha'] or data['pr']!=ctx['pr'] or type(data['pr']) not in {int,type(None)}:raise Invalid()","if False:raise Invalid()"),("if data['runner_nonce']!=ctx['nonce'] or not hex_value(data['log_sha256'],64):raise Invalid()","if False:raise Invalid()")],'round3-runner.test.js'),
 ('runner-log-hash','runner.py',[("if data['log_sha256']!=self.hash:raise Invalid()","if False:raise Invalid()"),("if self.cursor!=data['log_cursor'] or self.hash!=data['log_sha256']:raise Invalid()","if False:raise Invalid()")],'round3-runner.test.js'),
 ('runner-version','runner.py',[("if type(data[key]) is not type(manifest[key]) or data[key]!=manifest[key]:raise Invalid()","if False:raise Invalid()")],'round3-runner.test.js'),
 ('runner-redirect','runner.py',[("if reply.status in {301,302,303,307,308} or",'if False or'),("return reply.status,strict_json(reply.body)","return 200,strict_json(reply.body)")],'round3-runner.test.js'),
 ('runner-deadline','runner.py',[("with deadline(20):","with contextlib.nullcontext():")],'round3-runner.test.js'),
 ('runner-display-budget','runner.py',[("if self.displayed>=65000 or self.display_lines>=200:","if False:"),("max_bytes=max(0,65000-self.displayed-256),max_lines=max(0,199-self.display_lines)","max_bytes=65536,max_lines=200")],'round3-runner.test.js'),
 ('nonce-response','src/index.js',[("runner_nonce:r.runnerNonce??''","runner_nonce:''")],'round3-binding.test.js'),
 ('rent-head-binding','src/index.js',[("if(rentRequest.tested_sha!==target.sha||rentRequest.pr_number!==target.pr)","if(false)")],'round3-binding.test.js'),
 ('box-result-binding','src/index.js',[("if(payload.kind==='result'&&(payload.result.runner_nonce!==r.runnerNonce||payload.result.sha!==r.sha))","if(false)")],'round3-binding.test.js'),
 ('log-chain','src/ledger.js',[("record.logHash=await logChain(record.logHash??'0'.repeat(64),chunk)","record.logHash='0'.repeat(64)")],'round3-binding.test.js'),
 ('nonce-ingress','src/index.js',[("path==='/rent'?512:0","0")],'round3-binding.test.js'),
 ('nonce-cache','src/index.js',[("if(cached&&!force)refuse('UNAVAILABLE_BUDGET',409);","if(cached&&!force){const r=await ledger.get('record:'+cached.key);if(r?.verdict&&r.ended!==null){await ledger.grant(key,r,target.pr);return json(expose(r,key,target.pr));}refuse();}")],'round3-binding.test.js'),
]
def main():
    ap=argparse.ArgumentParser();ap.add_argument('--results',type=Path);ap.add_argument('--filter',default='');args=ap.parse_args();rows=[]
    for name,file,patches,test in CASES:
        if args.filter and args.filter not in name:continue
        p=BASE/file;original=p.read_text();changed=original
        for before,after in patches:
            assert changed.count(before)==1,(name,before);changed=changed.replace(before,after)
        try:
            p.write_text(changed)
            syntax=subprocess.run(['node','--check',str(p)] if p.suffix in {'.js','.mjs'} else ['python3','-c','import ast,sys;ast.parse(open(sys.argv[1]).read())',str(p)],capture_output=True)
            assert syntax.returncode==0,name+': syntax'
            r=subprocess.run(['node','--test',str(BASE/'test'/test)],capture_output=True,text=True,timeout=60)
            killers=re.findall(r'^not ok \d+ - (.*)',r.stdout,re.M)
            assert r.returncode and len(killers)>=2,(name,killers,r.stdout[-1500:])
            rows.append({'mutant':name,'count':len(killers),'killers':killers});print(name+': '+str(len(killers))+' killers',flush=True)
        finally:p.write_text(original)
    if args.results:args.results.write_text(json.dumps(rows,indent=2)+'\n')
if __name__=='__main__':main()
