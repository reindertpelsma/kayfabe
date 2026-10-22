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

CASES += [
 ('runner-drop-count','runner.py',[("kept+=len(safe.encode())+int(had_lf)","kept+=len(safe.encode())+1")],'round3-drop-count.test.js'),
 ('runner-API-prefix','runner.py',[("origin+('/'+prefix if prefix else '')+route","origin+route")],'secret-client.test.js'),
 ('runner-hostname-masks','runner.py',[(",*urllib.parse.urlsplit(origin).hostname.split('.')]:","]:")],'secret-client.test.js'),
 ('runner-audience-mask','runner.py',[(";print('::add-mask::'+audience,flush=True)","")],'secret-client.test.js'),
 ('run-idempotence','src/index.js',[("if(prior){if(prior.runnerNonce","if(false){if(prior.runnerNonce")],'broker.test.js'),
 ('runner-terminal-grace','runner.py',[("terminal=True;result=","terminal=False;result=")],'client.test.js'),

 ('runner-fresh-nonce','runner.py',[("'nonce':secrets.token_hex(32)","'nonce':'0'*64")],'secret-client.test.js'),
 ('diagnostic-binding','smoke.py',[("data.get('runner_nonce')!=self.runner_nonce or data.get('sha')!=tested_sha","False")],'round3-smoke-protocol.test.js'),
 ('diagnostic-reserve','smoke.py',[("credit>=8+(price or .5)*21/60","credit>=5+(price or .5)*21/60")],'round3-smoke-protocol.test.js'),
 ('diagnostic-BASE-claims','native-identity.mjs',[("workflow_ref:repo","obsolete_ref:repo"),("workflow_sha:'d'","obsolete_sha:'d'")],'round3-smoke-protocol.test.js'),
 ('runner-summary-fields','runner.py',[("if self.metadata is not None:","if False:")],'round3-summary.test.js'),
 ('target-base-binding','src/github.js',[("c.sha!==r.head_sha||c.workflow_sha!==r.head_sha||",'')],'round3-isolation.test.js'),
 ('target-PR-membership','src/github.js',[("if(member?.length!==1)refuse('UNAUTHORIZED',401);\n      p=await this.pr(hint.pr_number)","if(false)refuse('UNAUTHORIZED',401);\n      p=await this.pr(hint.pr_number)")],'round3-isolation.test.js'),
 ('runner-local-redaction','runner.py',[("if secret:text=text.replace(secret,'[redacted]')","if False:text=text.replace(secret,'[redacted]')")],'round3-display.test.js'),
 ('runner-C1-sequences','runner.py',[(r"(?:\x1b\]|\x9d)",r"(?:\x1b\])"),(r"(?:\x1b[P^_]|[\x90\x98\x9e\x9f])",r"(?:\x1b[P^_])")],'round3-display.test.js'),
 ('runner-isolated-env','../../.github/workflows/gpu-vast.yml',[("env -i PATH=/usr/bin:/bin","env PATH=/usr/bin:/bin")],'round3-bootstrap.test.js'),
 ('runner-shell-tracing','../../.github/workflows/gpu-vast.yml',[("          set +x","          set -x")],'round3-bootstrap.test.js'),
 ('base-trigger','../../.github/workflows/gpu-vast.yml',[("  pull_request_target:","  pull_request:")],'workflow.test.js'),
 ('OIDC-workflow-ref','src/index.js',[("||c.workflow_ref!==cfg.repo+'/.github/workflows/gpu-vast.yml@refs/heads/master'","")],'round3-isolation.test.js'),
 ('release-same-SHA','release_gate.py',[("or status['sha']!=sha",''),("and run['head_sha']==sha",'')],'round3-release.test.js'),
 ('release-origin','release_gate.py',[("if latest['state']!='success' or latest['creator']['login']!='github-actions[bot]' or latest['creator']['id']!=41898282:return False","if latest['state']!='success':return False"),("if run['id']!=int(match[1]) or run['path']!='.github/workflows/gpu-vast.yml' or run['status']!='completed' or run['conclusion']!='success':return False","if run['id']!=int(match[1]) or run['status']!='completed' or run['conclusion']!='success':return False"),("if run['repository']['full_name']!=REPO or run['head_repository']['full_name']!=REPO:return False","if False:return False")],'round3-release.test.js'),
 ('release-conclusion','release_gate.py',[("if latest['state']!='success' or",'if False or'),("or run['conclusion']!='success'",'')],'round3-release.test.js'),
 ('bundle-manifest','bundle.js',[("createHash('sha256').update(output).digest('hex')","'0'.repeat(64)")],'round3-release.test.js'),
 ('runner-cursor-race','runner.py',[("if data['log_increment_sanitised']!=''.join(chunks):raise Invalid()","if data['log_increment_sanitised']!=''.join(chunks) or data['next_cursor']>target:raise Invalid()")],'round3-runner.test.js'),
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
            if p.suffix not in {'.yml','.yaml'}:assert syntax.returncode==0,name+': syntax'
            r=subprocess.run(['node','--test',str(BASE/'test'/test)],capture_output=True,text=True,timeout=60)
            killers=re.findall(r'^not ok \d+ - (.*)',r.stdout,re.M)
            assert r.returncode and len(killers)>=2,(name,killers,r.stdout[-1500:])
            rows.append({'mutant':name,'count':len(killers),'killers':killers});print(name+': '+str(len(killers))+' killers',flush=True)
        finally:p.write_text(original)
    if args.results:args.results.write_text(json.dumps(rows,indent=2)+'\n')
if __name__=='__main__':main()
