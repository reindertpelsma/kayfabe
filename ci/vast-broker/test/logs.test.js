import test from 'node:test';
import assert from 'node:assert/strict';
import {setup,sha,nonce,finalLine} from './helpers.js';
const sanitize=async()=>await import('../src/sanitize.js');
for(const v of ["$(id)",'`id`',"'",'\n','a'.repeat(39),'A'.repeat(40)])test('onstart rejects SHA injection '+JSON.stringify(v),async()=>{
 const {onstart}=await import('../src/onstart.js');assert.throws(()=>onstart(v,nonce,150));
});
for(const v of ['$()','\n',"'",'b'.repeat(31),'B'.repeat(32)])test('onstart rejects nonce injection '+JSON.stringify(v),async()=>{
 const {onstart}=await import('../src/onstart.js');assert.throws(()=>onstart(sha,v,150));
});
test('onstart embeds only constrained SHA and nonce',async()=>{const {onstart}=await import('../src/onstart.js');const s=onstart(sha,nonce,150);assert.match(s,/git checkout --detach/);assert.match(s,/rev-parse HEAD/);assert.match(s,/vast_box_main.sh/);assert.match(s,/KF_RESULT/);assert.doesNotMatch(s,/authorized_keys|ssh_key|API_KEY/);});
for(const prefix of ['::error::hostile','::set-output name=x::bad','##[error]hostile'])test('sanitizer neutralizes command prefix '+prefix,async()=>{
 const {sanitizeLog}=await sanitize();const s=sanitizeLog(new TextEncoder().encode(prefix+'\n'));assert.ok(s.startsWith('[box] '));assert.ok(s.endsWith('\n'));
});
test('sanitizer removes ANSI and unsafe controls',async()=>{const {sanitizeLog}=await sanitize();assert.equal(sanitizeLog(new TextEncoder().encode('\x1b[31mred\x1b[0m\x00\r\x07\t\n')),'red\t\n');});
test('sanitizer forces valid UTF8',async()=>{const {sanitizeLog}=await sanitize();const s=sanitizeLog(Uint8Array.of(255,254));assert.equal(s,'��');});
test('sanitizer caps head and tail by UTF8 bytes',async()=>{const {sanitizeLog}=await sanitize();const s=sanitizeLog(new TextEncoder().encode('HEAD'+'é'.repeat(9000)+'TAIL'),1000);assert.ok(s.startsWith('HEAD'));assert.ok(s.endsWith('TAIL'));assert.match(s,/bytes dropped/);assert.ok(Buffer.byteLength(s)<=1000);assert.doesNotMatch(s,/�/);});
test('sanitizer redacts configured secret values',async()=>{const {sanitizeLog}=await sanitize();assert.equal(sanitizeLog(new TextEncoder().encode('binding-secret'),8192,['binding-secret']),'[redacted]');});
for(const mode of ['valid','blank-tail'])test('nonce result accepts '+mode,async()=>{const {parseResult}=await sanitize();assert.equal(parseResult(new TextEncoder().encode(finalLine()+(mode==='blank-tail'?'\n\n':'')),nonce),'PASS');});
for(const [name,data] of [ ['wrong-nonce',finalLine('PASS','d'.repeat(32))],['duplicate',finalLine()+finalLine()],['trailing',finalLine()+'more\n'],['hash',`KF_RESULT ${nonce} PASS nohash\n`],['ansi','\x1b[31m'+finalLine()],['prefix','x'+finalLine()],['no-newline',finalLine().trimEnd()],['invalid-utf8',new Uint8Array([255,...new TextEncoder().encode(finalLine())])]]){
 test('raw result rejects '+name,async()=>{const {parseResult}=await sanitize();assert.equal(parseResult(typeof data==='string'?new TextEncoder().encode(data):data,nonce),null);});
}
test('pending async URL is revisited without new upload request',async()=>{const h=await setup();await h.call();h.cloud.logReady=false;await h.call('/status/900');await h.call('/status/900');assert.equal(h.cloud.calls.filter(c=>c.url.includes('request_logs')).length,1);h.cloud.logReady=true;h.cloud.logText=finalLine();assert.equal((await (await h.call('/status/900')).json()).verdict,'PASS');});
for(const url of ['https://evil.example/log','http://s3.amazonaws.com/vast.ai/instance_logs/mock.log','https://s3.amazonaws.com/foreign/log','https://user@s3.amazonaws.com/vast.ai/instance_logs/x.log','https://s3.amazonaws.com/vast.ai/instance_logs/x.log?token=x'])test('rejects unsafe log URL '+url,async()=>{const h=await setup();await h.call();h.cloud.logUrl=url;assert.notEqual((await h.call('/status/900')).status,200);assert.equal(h.cloud.deletes.length,0);});
test('log download carries no credential headers',async()=>{const h=await setup();await h.call();h.cloud.logText=finalLine();await h.call('/status/900');const c=h.cloud.calls.find(c=>c.url.startsWith('https://s3.amazonaws.com'));assert.equal(c.headers.has('Authorization'),false);});
test('oversized log cannot pass a trailing valid marker',async()=>{const h=await setup();await h.call();h.cloud.logText='x'.repeat(1048576)+finalLine();assert.notEqual((await (await h.call('/status/900')).json()).verdict,'PASS');});
test('redirected log response cannot pass',async()=>{const h=await setup();await h.call();h.cloud.logStatus=302;h.cloud.logHeaders={Location:'https://evil.example'};h.cloud.logText=finalLine();assert.notEqual((await (await h.call('/status/900')).json()).verdict,'PASS');});
for(const control of ['\u200b','\u202e'])test('Unicode format control is removed before command neutralization '+control.codePointAt(0),async()=>{const {sanitizeLog}=await sanitize();assert.equal(sanitizeLog(new TextEncoder().encode(control+'::error::bad')),'[box] ::error::bad');});
for(const command of ['::error::','##[error]'])test('excerpt tail cannot introduce a bare workflow command '+command,async()=>{const {sanitizeLog}=await sanitize();const input='HEAD'+'x'.repeat(2000)+command+'x'.repeat(450-command.length);const log=sanitizeLog(new TextEncoder().encode(input),1000);assert.doesNotMatch(log,/^(::|##\[)/m);assert.ok(Buffer.byteLength(log)<=1000);});
for(const secret of ['ghs'+'_'+'A'.repeat(36),'github'+'_pat_'+'B'.repeat(40)])test('sanitizer redacts token-shaped text without retaining a request '+secret.slice(0,4),async()=>{const {sanitizeLog}=await sanitize();assert.equal(sanitizeLog(new TextEncoder().encode('prefix '+secret+' suffix')),'prefix [redacted] suffix');});
