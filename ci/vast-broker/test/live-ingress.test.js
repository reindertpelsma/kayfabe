import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
// FALSIFIER: either callback route performs an extra provider request. Those
// probes caused real 429s during a fast log burst; timestamps must reuse samples.
for (const route of ['report', 'kill'])
  test('live ' + route + ' ingress reuses timestamped provider sample without API probe', () => {
    const script = String.raw`
import http from 'node:http';import fs from 'node:fs';import os from 'node:os';import path from 'node:path';
const root=fs.mkdtempSync(path.join(os.tmpdir(),'kf-ingress-test-'));
let probes=0;globalThis.fetch=async()=>{probes++;throw Error('duplicate provider probe');};
fs.writeFileSync(root+'/.dev.vars','OWNER_STOP_TOKEN=mock-stop\nVAST_API_KEY=mock-key\nWORKER_URL=https://broker.example\nBROKER_PATH_PREFIX=test-prefix\n');
for(const [name,value] of Object.entries({'run.json':{key:'123-1'},'owned.json':{id:900},'allowed.json':{ip:'192.0.2.1'},'last-provider.json':{at:Date.now()-1000,actual_status:'running',cur_state:'running',intended_status:'running'}}))fs.writeFileSync(root+'/'+name,JSON.stringify(value));
const servers=[],ports={};const listen=http.Server.prototype.listen;
http.Server.prototype.listen=function(port,host,callback){servers.push(this);this.on('listening',()=>{ports[port]=this.address().port;});return listen.call(this,0,host,callback);};
const upstream=http.createServer((_req,res)=>res.end('{}'));await new Promise(resolve=>upstream.listen(1,'127.0.0.1',resolve));
const request=http.request;http.request=function(options,callback){return request.call(http,options.port===18787?{...options,port:upstream.address().port}:options,callback);};
process.argv[2]=root;await import(process.argv[1]);
while(!ports[18791]||!ports[18792])await new Promise(resolve=>setTimeout(resolve,5));
const status=await new Promise((resolve,reject)=>{const r=request({hostname:'127.0.0.1',port:ports[18791],path:'/test-prefix/'+process.argv[3]+'/123-1',method:'POST',headers:{'cf-connecting-ip':'192.0.2.1'}},response=>{response.resume();response.on('end',()=>resolve(response.statusCode));});r.on('error',reject);r.end('{}');});
const rows=fs.readFileSync(root+'/events.jsonl','utf8').trim().split('\n').map(JSON.parse);
for(const server of servers)await new Promise(resolve=>server.close(resolve));
fs.rmSync(root,{recursive:true});
console.log(JSON.stringify({status,probes,sample:rows.find(r=>r.kind==='callback_arrival')}));
`;
    const p = spawnSync(
      process.execPath,
      [
        '--input-type=module',
        '-e',
        script,
        new URL('../live/ingress.mjs', import.meta.url).href,
        'unused',
        route,
      ],
      { encoding: 'utf8', timeout: 10000 },
    );
    assert.equal(p.status, 0, p.stderr);
    const row = JSON.parse(p.stdout.trim().split('\n').at(-1));
    assert.equal(row.status, 200);
    assert.equal(row.probes, 0);
    assert.equal(row.sample.actual_status, 'running');
    assert.ok(row.sample.provider_sample_age_ms >= 1000);
  });
