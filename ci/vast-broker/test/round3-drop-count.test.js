import test from 'node:test';import assert from 'node:assert/strict';import {spawnSync} from 'node:child_process';
for(const [raw,limit,expected] of [['safe\u0000',100,1],['A'.repeat(50),10,40]])test('R3 drop marker excludes synthetic newline '+expected,()=>{
 const code=`import importlib.util,sys,json;s=importlib.util.spec_from_file_location('r','runner.py');m=importlib.util.module_from_spec(s);sys.modules['r']=m;s.loader.exec_module(m);safe,dropped=m.scrub(json.loads(sys.argv[1]),max_line=int(sys.argv[2]));assert dropped==int(sys.argv[3]),(safe,dropped);assert '['+sys.argv[3]+' bytes dropped by runner]' in safe`;
 const p=spawnSync('python3',['-I','-c',code,JSON.stringify(raw),String(limit),String(expected)],{cwd:new URL('../',import.meta.url),encoding:'utf8',timeout:5000});assert.equal(p.status,0,p.stderr);
});
