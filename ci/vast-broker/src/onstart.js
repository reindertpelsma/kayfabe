import {hexSha} from './policy.js';
import {validPathPrefix} from './routes.js';
export function onstart(sha,nonce,minutes,repo='reindertpelsma/kayfabe',callback){
  if(!hexSha(sha)||typeof nonce!=='string'||!/^[0-9a-f]{32}$/.test(nonce)||!Number.isInteger(minutes)||minutes<1||minutes>60||!/^[-\w.]+\/[-\w.]+$/.test(repo))throw Error('Invalid fixed onstart inputs');
  let origin;try{const u=new URL(callback.url);if(u.protocol!=='https:'||u.username||u.password||u.search||u.hash||u.pathname!=='/')throw Error();origin=u.origin;}catch{throw Error('Invalid callback origin');}
  if(!/^[1-9][0-9]{0,19}-[1-9][0-9]{0,19}$/.test(callback.run)||!/^v1\.[1-9][0-9]{0,19}\.[1-9][0-9]{0,19}\.[1-9][0-9]{0,12}\.[A-Za-z0-9_-]{43}$/.test(callback.token)||callback.token.split('.').slice(1,3).join('-')!==callback.run)throw Error('Invalid callback capability');
  const prefix=callback.path??'';if(prefix!==''&&!validPathPrefix(prefix))throw Error('Invalid callback prefix');
  const grace=callback.graceMinutes??5;if(!Number.isInteger(grace)||grace<1||grace>15)throw Error('Invalid grace');
  if(callback.runnerNonce!==undefined&&!/^[0-9a-f]{64}$/.test(callback.runnerNonce))throw Error('Invalid runner nonce');
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
python3 - <<'KF_REPORT'
import json,math,os,select,subprocess,time
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
    if kind!='result' and (attempted>=600 or sent+len(body)>900000):
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
work=r'''exec timeout --signal=TERM --kill-after=30s ${Math.max(1,minutes*60-60)}s bash -s <<'KF_TEST'
set -euo pipefail
git clone --no-checkout https://github.com/${repo}.git /workspace/kf-ci-src
cd /workspace/kf-ci-src
git fetch origin ${sha}
git checkout --detach ${sha}
[ "$(git rev-parse HEAD)" = '${sha}' ]
bash scripts/ci/vast_box_main.sh '${sha}'
KF_TEST
'''
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
        elif current-heartbeat>=60:
            post('progress','Suite running');heartbeat=current
    if pending: post('log',pending.decode('utf-8',errors='replace'))
rc=child.wait()
with open(backup,'rb') as local:
    local.seek(0,os.SEEK_END);local.seek(max(0,local.tell()-8192))
    tail=local.read(8192).decode('utf-8',errors='replace')
ack=post('result','Suite complete; sender omitted '+str(omitted)+' raw bytes beyond its bounded transport allowance.',{'nonce':nonce,'verdict':'PASS' if rc==0 else 'FAIL',**({'runner_nonce':runner_nonce,'sha':'${sha}'} if runner_nonce else {})})
seconds=ack.get('grace_remaining_seconds') if isinstance(ack,dict) else None
remaining=str(math.ceil(seconds/60)) if type(seconds) is int and 0<=seconds<=${grace*60} else 'unknown'
print('Use kf-box-kill [reason] to end this box early; configured maximum ${grace} minutes; Worker reports '+remaining+' minutes remaining (hard TTL included).')
raise SystemExit(rc)
KF_REPORT
`;
}
