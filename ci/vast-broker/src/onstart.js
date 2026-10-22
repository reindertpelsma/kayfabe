import {DEFAULTS} from './defaults.js';
import {hexSha} from './policy.js';
import {validPathPrefix} from './routes.js';
export function onstart(sha,nonce,minutes,repo='reindertpelsma/kayfabe',callback){
  if(!hexSha(sha)||typeof nonce!=='string'||!/^[0-9a-f]{32}$/.test(nonce)||!Number.isInteger(minutes)||minutes<1||minutes>DEFAULTS.maxMinutes||!/^[-\w.]+\/[-\w.]+$/.test(repo))throw Error('Invalid fixed onstart inputs');
  let origin;try{const u=new URL(callback.url);if(u.protocol!=='https:'||u.username||u.password||u.search||u.hash||u.pathname!=='/')throw Error();origin=u.origin;}catch{throw Error('Invalid callback origin');}
  if(!/^[1-9][0-9]{0,19}-[1-9][0-9]{0,19}$/.test(callback.run)||!/^v1\.[1-9][0-9]{0,19}\.[1-9][0-9]{0,19}\.[1-9][0-9]{0,12}\.[A-Za-z0-9_-]{43}$/.test(callback.token)||callback.token.split('.').slice(1,3).join('-')!==callback.run)throw Error('Invalid callback capability');
  const prefix=callback.path??'';if(prefix!==''&&!validPathPrefix(prefix))throw Error('Invalid callback prefix');
  const grace=callback.graceMinutes??DEFAULTS.failGraceMinutes;if(!Number.isInteger(grace)||grace<1||grace>15)throw Error('Invalid grace');
  if(callback.runnerNonce!==undefined&&!/^[0-9a-f]{64}$/.test(callback.runnerNonce))throw Error('Invalid runner nonce');
  if(callback.softDeadline!==undefined&&(!Number.isSafeInteger(callback.softDeadline)||callback.softDeadline<=0))throw Error('Invalid soft deadline');
  return `#!/bin/bash
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
command -v curl >/dev/null && command -v python3 >/dev/null && command -v git >/dev/null || { apt-get update -qq; apt-get install -y -qq curl python3 git ca-certificates; }
install -d -m 700 /workspace
cat > /usr/local/bin/kf-box-kill <<'KF_KILL'
#!/bin/sh
set -eu
# Only this run's limited capability; neither account nor owner credentials.
exec curl --silent --show-error --fail --max-time 20 --proto '=https' --request POST --header 'Authorization: Bearer ${callback.token}' --data-binary "\${1:-box-request}" '${origin}${prefix?'/'+prefix:''}/kill/${callback.run}'
KF_KILL
chmod 700 /usr/local/bin/kf-box-kill
cat > /usr/local/bin/kf-box-hold <<'KF_HOLD'
#!/bin/sh
set -eu
# This run only; cannot remove its hard limit or access account credentials.
exec curl --silent --show-error --fail --max-time 20 --proto '=https' --request POST --header 'Authorization: Bearer ${callback.token}' --data-binary '' '${origin}${prefix?'/'+prefix:''}/hold/${callback.run}'
KF_HOLD
chmod 700 /usr/local/bin/kf-box-hold
cat > /etc/profile.d/kf-ci-hold.sh <<'KF_PROFILE'
# An authenticated SSH login takes MANUAL; ordinary non-login workloads do not.
if [ -n "\${SSH_CONNECTION:-}" ]; then
    /usr/local/bin/kf-box-hold >/dev/null 2>&1 || true
fi
KF_PROFILE
chmod 644 /etc/profile.d/kf-ci-hold.sh
python3 - <<'KF_REPORT'
import json,math,os,re,select,subprocess,time
url='${origin}${prefix?'/'+prefix:''}/report/${callback.run}'
token='${callback.token}'
nonce='${nonce}'
runner_nonce='${callback.runnerNonce??''}'
seq=0
sent=0
attempted=0
omitted=0
backup='/workspace/kf-ci-output.log'
def post(kind,text=None,result=None):
    global seq,sent,attempted,omitted
    payload={'seq':seq+1,'kind':kind}
    if text is not None: payload['text']=text
    if result is not None: payload['result']=result
    body=json.dumps(payload,separators=(',',':'),ensure_ascii=True).encode()
    heartbeat_post=kind=='log' and text==''
    if kind!='result' and not heartbeat_post and (attempted>=600 or sent+len(body)>900000):
        omitted+=len((text or '').encode());return
    seq+=1;sent+=len(body);attempted+=1
    # curl retries reuse this exact payload/sequence; no redirect or arbitrary URL.
    response=subprocess.run(['curl','--silent','--show-error','--fail','--max-time','10',
        '--retry','2','--retry-delay','1','--retry-all-errors','--proto','=https',
        '--request','POST','--header','Content-Type: application/json',
        '--header','Authorization: Bearer '+token,'--data-binary','@-',url],
        input=body,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,check=False)
    if kind=='result' and response.returncode==0:
        try: return json.loads(response.stdout)
        except (ValueError,TypeError): return None
work=r'''exec timeout --signal=TERM --kill-after=30s __KF_SOFT_SECONDS__s bash -s <<'KF_TEST'
set -euo pipefail
find /workspace -maxdepth 1 -name kf-ci-result.json -delete
git clone --no-checkout https://github.com/${repo}.git /workspace/kf-ci-src
cd /workspace/kf-ci-src
git fetch origin ${sha}
git checkout --detach ${sha}
[ "$(git rev-parse HEAD)" = '${sha}' ]
export KF_SOFT_DEADLINE_MS=__KF_SOFT_DEADLINE_MS__
bash scripts/ci/vast_payload.sh '${sha}' '${nonce}'
KF_TEST
'''
soft_deadline=${callback.softDeadline??'None'}
remaining=math.floor(soft_deadline/1000-time.time()) if soft_deadline is not None else ${minutes*60-DEFAULTS.softMarginMinutes*60}
work=work.replace('__KF_SOFT_SECONDS__',str(max(1,remaining))) if remaining>0 else 'exit 124\\n'
work=work.replace('__KF_SOFT_DEADLINE_MS__',str(soft_deadline if soft_deadline is not None else math.floor(time.time()*1000+remaining*1000)))
post('progress','Suite starting')
child=subprocess.Popen(['bash','-s'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
child.stdin.write(work.encode());child.stdin.close()
pending=b'';last=time.monotonic();heartbeat=last
with open(backup,'wb') as local:
    while True:
        ready,_,_=select.select([child.stdout],[],[],1)
        if ready:
            data=os.read(child.stdout.fileno(),8192)
            if not data: break
            local.write(data);local.flush()
            pending+=data
        current=time.monotonic()
        if pending and (len(pending)>=8192 or current-last>=1):
            post('log',pending.decode('utf-8',errors='replace'));pending=b'';last=current
        if current-heartbeat>=${DEFAULTS.heartbeatMinutes*60}:
            post('log','');heartbeat=current
    if pending: post('log',pending.decode('utf-8',errors='replace'))
rc=child.wait()
with open(backup,'rb') as local:
    local.seek(0,os.SEEK_END);local.seek(max(0,local.tell()-8192))
    tail=local.read(8192).decode('utf-8',errors='replace')
# A process exit or log text cannot supply a GPU verdict. Consume only the
# bounded structured file written by the approved payload, binding SHA + nonce.
expected={'provision':1,'kf_tests':None,'gates':9,'kf3_build':1,'raw_suite':30,'thin_suite':30,'birth_census':1,'cuda_host':4,'cuda_fat':4}
errors={'preflight','timeout','infrastructure','merge_bar','app_exit','xid'}
unknown={'gpu_name':'unknown','compute_cap':0,'driver_version':'unknown'}
result={'nonce':nonce,'sha':'${sha}','verdict':'FAIL','suites':[{'name':'infrastructure','pass':0,'fail':1,'skip':0}],'hardware':unknown}
def unique(items):
    out={}
    for k,v in items:
        if k in out:raise ValueError('duplicate field')
        out[k]=v
    return out
try:
    with open('/workspace/kf-ci-result.json','rb') as source:raw=source.read(65537)
    if len(raw)>65536:raise ValueError('size')
    candidate=json.loads(raw.decode('utf-8',errors='strict'),object_pairs_hook=unique)
    if type(candidate) is not dict or set(candidate)!={'nonce','sha','verdict','suites','hardware'} or candidate['nonce']!=nonce or candidate['sha']!='${sha}' or candidate['verdict'] not in {'PASS','FAIL','UNAVAILABLE'}:raise ValueError('binding')
    rows=candidate['suites'];hardware=candidate['hardware']
    if type(rows) is not list or len(rows)>19 or any(type(r) is not dict or set(r)!={'name','pass','fail','skip'} or r['name'] not in set(expected)|errors or any(type(r[k]) is not int or not 0<=r[k]<=10000000 for k in ['pass','fail','skip']) for r in rows) or len({r['name'] for r in rows})!=len(rows):raise ValueError('suites')
    if type(hardware) is not dict or set(hardware)!={'gpu_name','compute_cap','driver_version'} or not isinstance(hardware['gpu_name'],str) or not re.fullmatch(r'[A-Za-z0-9_. -]{1,100}',hardware['gpu_name']) or type(hardware['compute_cap']) is not int or not 0<=hardware['compute_cap']<=2000 or not isinstance(hardware['driver_version'],str) or not re.fullmatch(r'unknown|[0-9]{3}\\.[0-9]{1,3}(?:\\.[0-9]{1,2})?',hardware['driver_version']):raise ValueError('hardware')
    complete=len(rows)==len(expected) and {r['name'] for r in rows}==set(expected) and all(r['fail']==0 and r['skip']==0 and r['pass']>0 and (expected[r['name']] is None or r['pass']==expected[r['name']]) for r in rows)
    if rc!=0 or candidate['verdict']!='PASS' or not complete or hardware['gpu_name']=='unknown' or hardware['driver_version']=='unknown' or hardware['compute_cap']<750:
        candidate['verdict']='FAIL'
        if not any(r['fail'] for r in rows):rows.append({'name':'timeout' if rc==124 else 'app_exit','pass':0,'fail':1,'skip':0})
    result=candidate
except (ValueError,TypeError,KeyError,OSError,UnicodeError):pass
if rc==124 and not any(r['name']=='timeout' for r in result['suites']):result['suites'].append({'name':'timeout','pass':0,'fail':1,'skip':0})
if runner_nonce:result['runner_nonce']=runner_nonce
ack=post('result','Suite complete; sender omitted '+str(omitted)+' raw bytes beyond its bounded transport allowance.',result)
seconds=ack.get('grace_remaining_seconds') if isinstance(ack,dict) else None
remaining=str(math.ceil(seconds/60)) if type(seconds) is int and 0<=seconds<=${grace*60} else 'unknown'
print('Use kf-box-kill [reason] to end this box early; configured maximum ${grace} minutes; Worker reports '+remaining+' minutes remaining (hard TTL included).')
raise SystemExit(rc if rc else (0 if result['verdict']=='PASS' else 1))
KF_REPORT
`;
}
