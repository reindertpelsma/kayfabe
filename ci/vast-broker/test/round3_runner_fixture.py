import hashlib,importlib.util,io,json,sys
from pathlib import Path
spec=importlib.util.spec_from_file_location('runner',Path(__file__).resolve().parents[1]/'runner.py')
m=importlib.util.module_from_spec(spec);sys.modules['runner']=m;spec.loader.exec_module(m)
case=sys.argv[1]
ctx={'sha':'a'*40,'run':'123-1','pr':7,'nonce':'f'*64}
manifest={'protocol_version':3,'worker_version':'v4-r3','bundle_sha256':'b'*64}
text={'controls':'safe\x1b[31mRED\x1b[0m\x1b]0;hidden\x07\u202eEVIL\u200b\x00\r\x7f\u0085é\n',
      'commands':'::set-output name=x::oops\n::add-mask::secret\n::error::x\n##[group]\n',
      'caps':'L'*100000+'\n'+'line\n'*10000,
      'display-only':'PASS\nstate=COMPLETE\nverdict=PASS\n'} .get(case,'ordinary bounded log\n')
encoded=text.encode();h=hashlib.sha256(bytes(32)+len(encoded).to_bytes(4,'big')+encoded).hexdigest()
verdict='FAIL' if case=='display-only' else 'PASS'
status={'state':'COMPLETE','verdict':verdict,'run':ctx['run'],'id':'900','sha':ctx['sha'],'pr':7,'cleanup_pending':True,'result_at':2000,'ended_at':None,'started_at':1000,'deadline_at':302000,'gpu_model':'RTX 4070','cost_estimate_usd':.01,'log_cursor':1,'log_tail_sanitised':text[:100],'bytes_dropped':0,'tail_bytes_dropped':0,'runner_nonce':ctx['nonce'],'log_sha256':h,**manifest}
if case=='extra':status['evil']='unexpected'
if case=='wrong-sha':status['sha']='c'*40
if case=='wrong-nonce':status['runner_nonce']='0'*64
if case=='wrong-run':status['run']='124-1'
if case=='bad-state':status['state']='::error::'
if case=='bad-verdict':status['verdict']='PAss'
if case=='wrong-hash':status['log_sha256']='0'*64
if case=='wrong-version':status['bundle_sha256']='0'*64
calls=[];printed=[];summary=io.StringIO()
def client(route,method='GET',body=None):
    calls.append((route,method,body))
    if route=='/rent':
        assert body['runner_nonce']==ctx['nonce'] and body['tested_sha']==ctx['sha']
        if case=='timeout':raise TimeoutError()
        raw=json.dumps(status,ensure_ascii=False).encode()
        if case=='duplicate':raw=raw[:-1]+b',"verdict":"PASS"}'
        if case=='invalid-utf8':raw=b'{"state":"\xff"}'
        if case=='huge':raw=b'X'*10000000
        if case=='html':raw=b'<html>200</html>'
        return m.Reply(302 if case=='redirect' else 200,{'content-type':'text/html' if case=='html' else 'application/json'},raw)
    if route.startswith('/logs/'):
        chunks=[text[i:i+4000] for i in range(0,len(text),4000)] if case=='caps' else [text]
        # The caps probe is intentionally one overlarge page: bounded fail-closed.
        data={'cursor':0,'next_cursor':len(chunks),'chunks':chunks,'log_increment_sanitised':text,'log_sha256':h,'bytes_dropped':0}
        return m.Reply(200,{'content-type':'application/json'},json.dumps(data,ensure_ascii=False).encode())
    return m.Reply(200,{'content-type':'application/json'},b'{}')
result=m.Runner(ctx,manifest,client,emit=printed.append,summary=summary).execute()
output=''.join(printed)+summary.getvalue()
if case in {'valid','controls','commands'}:
    assert result['verdict']=='PASS',result
elif case=='display-only':assert result['verdict']=='FAIL',result
else:assert result['verdict']=='UNAVAILABLE',result
assert result['state'] in m.STATES and result['verdict'] in m.VERDICTS
assert type(result['verified']) is int and result['verified'] in {0,1}
assert '\x1b' not in output and '\x00' not in output and '\r' not in output and '\u202e' not in output and '\u200b' not in output and '\x7f' not in output and '\u0085' not in output
assert '::set-output' not in output and '::add-mask' not in output and '::error' not in output and '##[' not in output
assert len(output.encode())<150000
if case=='caps':
    safe,dropped=m.scrub(encoded,max_bytes=4096,max_lines=20,max_line=100)
    assert dropped>0 and 'bytes dropped' in safe and len(safe.encode())<5000
if case in {'controls','commands'}:assert 'UNTRUSTED' in output
assert calls[0][2]['runner_nonce']==ctx['nonce']
