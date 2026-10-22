#!/usr/bin/env python3
"""Owner-authorized V4 round 2. REAL provider; local OIDC, tiny workload overlay.

Keep all runtime files private. No account credential is sent to the rented box.
Source overlays replace MODELS (local any-NVIDIA policy), the tiny test work
string plus a PASS-only delayed in-box kill, and, with --docker-smoke, the local
image/vm flag. Docker smoke never establishes production KVM workload validity.
Wrangler, identity, ingress and cron are controller processes; reapers are
independent sessions and may only be cancelled after exact-ID + label absence.
"""
import argparse,hashlib,json,os,re,shutil,signal,socket,subprocess,sys,time,urllib.parse
from pathlib import Path
from smoke import Client,Stop,configured_key,provider,request
HERE=Path(__file__).resolve().parent

def tiny_work(verdict):
    assert verdict in {'PASS','FAIL'}
    return """set -euo pipefail
echo 'TINY_TEST_START'
nvidia-smi --query-gpu=name --format=csv,noheader
df -h /
python3 - <<'TINY'
import hashlib
assert 2+2==4
assert hashlib.sha256(b'abc').hexdigest()=='ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
print('Bearer '+'FAKE'*12)
print('eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJwbGFudGVkIn0.'+'J'*80)
print('\\x1b[31mred\\x1b[0m')
print('::error::x')
print('##[group]')
print('Z'*100000)
print('TINY_TEST_EXIT %s')
TINY
exit %s
""" % (0 if verdict=='PASS' else 7,0 if verdict=='PASS' else 7)

def overlay(root,models,docker=False):
    shutil.copytree(HERE/'src',root/'src')
    policy=root/'src/policy.js';s=policy.read_text()
    s,n=re.subn(r'export const MODELS=\[[^\n]+?\];','export const MODELS='+json.dumps(models)+';',s);assert n==1
    policy.write_text(s)
    if docker:
        s=policy.read_text();s,n=re.subn(r"export const IMAGE='[^']+';","export const IMAGE='docker.io/ubuntu:22.04';",s);assert n==1;policy.write_text(s)
        vast=root/'src/vast.js';s=vast.read_text();assert s.count('vm:true')==1;vast.write_text(s.replace('vm:true','vm:false'))
    # Scenario is selected before Wrangler starts. The fixed reporting sender,
    # nonce/verdict authentication, sanitization and lifecycle are unchanged.
    for name in ['entry.mjs','ingress.mjs','reaper.py']:shutil.copy(HERE/'live'/name,root/name)
    shutil.copy(HERE/'native-identity.mjs',root/'identity.mjs')

def set_work(root,verdict):
    source=(HERE/'src/onstart.js').read_text()
    start=source.index("work=r'''" )+len("work=r'''");end=source.index("'''\npost('progress'",start)
    source=source[:start]+tiny_work(verdict)+source[end:]
    if verdict=='PASS':
        # Run the shipped shell/curl CLI from inside the real box, after it has
        # retained PASS for 30 seconds. No controller-side capability simulation.
        source=source.replace('\nKF_REPORT\n`;','\nKF_REPORT\nsleep 30\nkf-box-kill smoke-pass-complete\n`;')
    (root/'src/onstart.js').write_text(source)

def dump(path,value):path.write_text(json.dumps(value,indent=2)+'\n');path.chmod(0o600)
def spawn(root,name,argv):
    log=open(root/(name+'.log'),'ab');p=subprocess.Popen(argv,stdout=log,stderr=log,start_new_session=True,cwd=root);log.close();return p
def stop(p):
    if p and p.poll() is None:
        os.killpg(p.pid,signal.SIGTERM)
        try:p.wait(timeout=10)
        except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
def wait_for(test,seconds=30):
    until=time.monotonic()+seconds
    while time.monotonic()<until:
        try:
            if test():return
        except Exception:pass
        time.sleep(.25)
    raise Stop('LOCAL_STARTUP')
def exact_absence(root):
    owned=json.loads((root/'owned.json').read_text());run=json.loads((root/'run.json').read_text())
    detail=provider('/api/v0/instances/'+str(owned['id'])+'/')
    if 'instances' not in detail:raise Stop('PROVIDER_INSPECT_SHAPE')
    i=detail['instances']
    if i is not None:
        if i.get('id')!=owned['id'] or i.get('label')!=run['label']:raise Stop('OWNERSHIP')
        return False
    q=urllib.parse.urlencode({'limit':2,'select_filters':json.dumps({'label':{'eq':run['label']}})})
    d=provider('/api/v1/instances/?'+q)
    return d.get('success') is True and isinstance(d.get('instances'),list) and not d.get('next_token') and not any(x.get('label')==run['label'] for x in d['instances'])

def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--runtime',required=True,type=Path);ap.add_argument('--wrangler',required=True,type=Path)
    ap.add_argument('--docker-smoke',action='store_true',help='Owner round 2 local Docker/SSH tiny test; not production KVM verification')
    args=ap.parse_args();root=args.runtime.resolve();root.mkdir(mode=0o700,parents=True,exist_ok=True);root.chmod(0o700)
    os.umask(0o077);children=[];reaper=None;active=False;total=0;results=[]
    try:
        # Never treat another process's listener as our readiness signal.
        for port in [18787,18788,18791,18792]:
            with socket.socket() as probe:
                probe.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
                probe.bind(('127.0.0.1',port))
        query={'limit':20,'type':'ondemand','verified':{'eq':True},'vms_enabled':{'eq':True},'rentable':{'eq':True},'rented':{'eq':False},'num_gpus':{'eq':1},'reliability':{'gte':.99},'disk_space':{'gte':120},'dph_total':{'lte':.8},'allocated_storage':120,'order':[['dph_total','asc']]}
        offers=provider('/api/v0/bundles/','POST',query).get('offers',[])
        offers=[o for o in offers if str(o.get('gpu_name','')).startswith(('GTX ','RTX ')) and (o.get('verified') is True or o.get('verification')=='verified') and o.get('vms_enabled') is True and o.get('rentable') is True and o.get('rented') is False and o.get('num_gpus')==1 and o.get('reliability',0)>=.99 and o.get('disk_space',0)>=120 and 0<=o.get('dph_total',999)<=.8]
        if not offers:raise Stop('NO_ELIGIBLE_NVIDIA_OFFER')
        models=sorted({o['gpu_name'].replace(' ','_') for o in offers});overlay(root,models,args.docker_smoke)
        tunnel=spawn(root,'tunnel',['cloudflared','tunnel','--url','http://127.0.0.1:18791','--no-autoupdate']);children.append(tunnel)
        def tunnel_url():
            m=re.search(r'https://[a-z0-9-]+\.trycloudflare\.com',(root/'tunnel.log').read_text());return m[0] if m else None
        wait_for(tunnel_url,40);url=tunnel_url()
        import secrets
        env={'RENTING_ENABLED':'true','OWNER_GITHUB_ID':'42','REPOSITORY':'reindertpelsma/kayfabe','WORKER_URL':url,'VAST_API_KEY':configured_key(),'RUN_TOKEN_SECRET':secrets.token_urlsafe(40),'OWNER_STOP_TOKEN':secrets.token_urlsafe(40),'BROKER_PATH_PREFIX':secrets.token_urlsafe(32),'ALLOW_TEST_ISSUER':'true','TEST_ISSUER':'http://127.0.0.1:18788','MAX_PRICE_USD_HOUR':'0.8','TTL_MINUTES':'20','PROVISION_MINUTES':'10','POST_RESULT_GRACE_MINUTES':'5','DAILY_SPEND_USD':'1','ROLLING_SPEND_USD':'1','CREDIT_RESERVE_USD':'8'}
        (root/'.dev.vars').write_text(''.join(k+'='+v+'\n' for k,v in env.items()));(root/'.dev.vars').chmod(0o600)
        config='''name = "kayfabe-v4-round2-local"
main = "entry.mjs"
compatibility_date = "2026-10-21"
[[durable_objects.bindings]]
name = "LEDGER"
class_name = "BrokerLedger"
[[migrations]]
tag = "v1"
new_sqlite_classes = ["BrokerLedger"]
[triggers]
crons = ["* * * * *"]
'''
        (root/'wrangler.toml').write_text(config)
        identity=spawn(root,'identity',['node','identity.mjs']);children.append(identity)
        wait_for(lambda:request('http://127.0.0.1:18788/token')[0]==200)
        if identity.poll() is not None:raise Stop('IDENTITY_STARTUP')
        ingress=spawn(root,'ingress',['node','ingress.mjs',str(root)]);children.append(ingress)
        client=Client(env,'http://127.0.0.1:18787','http://127.0.0.1:18788',root)
        base=int(time.time()*1000)
        for index,verdict in enumerate(['PASS','FAIL']):
            if total+.8*15/60>=.6:raise Stop('ESTIMATED_STOP_CAP')
            run=str(base+index)+'-1';label='kf-ci-'+run
            for file in ['owned.json','intent.json','allowed.json','reaper-armed.json']:(root/file).unlink(missing_ok=True)
            dump(root/'run.json',{'key':run,'label':label})
            sha=hashlib.sha256(tiny_work(verdict).encode()).hexdigest()[:40]
            request(client.identity+'/select','POST',body={'run':run.split('-')[0],'sha':sha})
            set_work(root,verdict)
            wrangler=spawn(root,'wrangler-'+verdict,[str(args.wrangler),'dev','--config','wrangler.toml','--ip','127.0.0.1','--port','18787','--test-scheduled','--persist-to',str(root/'state')]);children.append(wrangler)
            wait_for(lambda:request(client.base+client.prefix+'/status/'+run)[0]==401,45)
            if wrangler.poll() is not None or ingress.poll() is not None:raise Stop('LISTENER_STARTUP')
            reaper=spawn(root,'reaper-'+verdict,[sys.executable,'reaper.py',str(root)])
            wait_for(lambda:(root/'reaper-armed.json').exists())
            started=time.time();active=True
            code,data=client.call('/rent','POST')
            dump(root/('rent-'+verdict+'.json'),{'status':code,'response':data})
            if code!=200:raise Stop('RENT_REFUSED_'+str(code))
            wait_for(lambda:(root/'owned.json').exists(),10)
            owned=json.loads((root/'owned.json').read_text());intent=json.loads((root/'intent.json').read_text());price=intent['price']
            print(json.dumps({'phase':verdict,'created':True,'hourly_usd':price,'reaper_armed':True}),flush=True)
            cursor=0;logs='';result_at=None;absent_at=None;last_cron=0;status=None;states=[]
            # Authorization negative control never receives the real run token.
            bad_code,_=client.call('/report/'+run,'POST','bad',{'seq':1,'kind':'result','result':{'nonce':'0'*32,'verdict':'PASS'}},oidc=False)
            assert bad_code==401
            while time.time()-started<720:
                now=time.time()
                detail=provider('/api/v0/instances/'+str(owned['id'])+'/')
                if 'instances' not in detail:raise Stop('PROVIDER_INSPECT_SHAPE')
                i=detail['instances']
                states.append({'after_create_s':round(now-started,3),'actual_status':i.get('actual_status') if i else None,'cur_state':i.get('cur_state') if i else None,'intended_status':i.get('intended_status') if i else None})
                if i is None:
                    if exact_absence(root):absent_at=time.time();active=False;break
                else:
                    if i.get('label')!=label:raise Stop('OWNERSHIP')
                    if i.get('public_ipaddr'):
                        import ipaddress
                        dump(root/'allowed.json',{'ip':str(ipaddress.ip_address(i['public_ipaddr']))})
                if now-last_cron>=60:client.scheduled();last_cron=now
                code,status=client.call('/status/'+run)
                if code==200:
                    cursor,increment=client.drain(run,cursor);logs+=increment
                    if status.get('result_at') is not None and result_at is None:
                        result_at=status['result_at']/1000
                        print(json.dumps({'phase':verdict,'structured_verdict':status['verdict'],'kept_for_grace':status['cleanup_pending']}),flush=True)
                    dump(root/('status-'+verdict+'.json'),status)
                estimated=total+price*(time.time()-started)/3600
                if estimated>=.6:raise Stop('ESTIMATED_STOP_CAP')
                time.sleep(3)
            if absent_at is None:raise Stop('API_ABSENCE_DEADLINE')
            stop(reaper);reaper=None
            code,status=client.call('/status/'+run)
            elapsed=absent_at-started;cost=price*elapsed/3600;total+=cost
            planted_clean=all(s not in logs for s in ['FAKE'*12,'J'*80,'\x1b','::','##[','Z'*100000]) and len(logs.encode())<=4194304
            planted_exercised='TINY_TEST_START' in logs and 'TINY_TEST_EXIT' in logs and '[redacted]' in logs
            observed={'phase':verdict,'live_provider':True,'create_to_absence_s':round(elapsed,3),'result_to_absence_s':round(absent_at-result_at,3) if result_at else None,'result_received':result_at is not None,'structured_verdict':status.get('verdict'),'api_id_and_label_absent':True,'estimated_usd':cost,'planted_clean':planted_clean,'planted_exercised':planted_exercised,'log_bytes':len(logs.encode()),'unauthenticated_result_status':bad_code,'provider_states':states}
            results.append(observed);dump(root/'results.json',{'results':results,'estimated_usd':total})
            (root/('rendered-'+verdict+'.txt')).write_text('UNTRUSTED output of the REAL rented box\n'+logs)
            if result_at is None or status.get('verdict')!=verdict:raise Stop('NO_EXPECTED_STRUCTURED_RESULT')
            if not planted_clean or not planted_exercised:raise Stop('PUBLIC_LOG_SCRUB')
            stop(wrangler)
        dump(root/'finished.json',{'state':'COMPLETE','results':results,'estimated_usd':total,'requests':client.rows})
        print(json.dumps({'state':'COMPLETE','estimated_usd':total,'live_runs':len(results)}),flush=True)
    except Exception as error:
        reason=str(error) if isinstance(error,Stop) else type(error).__name__
        dump(root/'failure.json',{'state':'STOP','reason':reason,'estimated_completed_usd':total})
        print(json.dumps({'state':'STOP','reason':reason}),flush=True)
        if active:
            # Worker owner-stop first; exact own-ID provider deletion is fallback.
            try:client.call('/stop/'+run,'POST',env['OWNER_STOP_TOKEN'],oidc=False)
            except Exception:pass
            try:
                if not exact_absence(root):
                    owned=json.loads((root/'owned.json').read_text());i=provider('/api/v0/instances/'+str(owned['id'])+'/')['instances']
                    if i and i['label']==label:provider('/api/v0/instances/'+str(owned['id'])+'/','DELETE')
                wait_for(lambda:exact_absence(root),30);active=False;stop(reaper);reaper=None
                dump(root/'emergency-cleanup.json',{'id_and_label_absent':True})
            except Exception:print('Independent reaper retained: absence unresolved.',flush=True)
        return 1
    finally:
        for child in reversed(children):stop(child)
        dump(root/'controller-cleanup.json',{'children_stopped':all(p.poll() is not None for p in children),'live_absence_unresolved':active,'independent_reaper_retained':bool(reaper and reaper.poll() is None)})
    return 0
if __name__=='__main__':raise SystemExit(main())
