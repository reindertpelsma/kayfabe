import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {spawnSync} from 'node:child_process';
import {sanitizeLog} from '../src/sanitize.js';
import {publicLogRenderer} from '../public-log.js';
// FALSIFIER: replacing adjacent delimiter pairs creates another raw delimiter
// across replacement boundaries, violating the global command-data boundary.
for(const text of ['::::error::::','::::::warning::::::']){
 test('overlapping command delimiters are neutralized by Worker '+text.length,()=>assert.doesNotMatch(sanitizeLog(new TextEncoder().encode(text)),/::|##\[/));
 test('overlapping command delimiters are neutralized by renderer '+text.length,async()=>{
  const dir=await mkdtemp(tmpdir()+'/v4-delimiter-');
  try{await writeFile(dir+'/input',JSON.stringify({log_increment_sanitised:'[box] '+text+'\n'}));
   const r=spawnSync('python3',['-c',publicLogRenderer,'live',dir+'/input',dir+'/unused'],{encoding:'utf8'});assert.equal(r.status,0,r.stderr);assert.doesNotMatch(r.stdout,/::|##\[/);assert.match(r.stdout,/\[box\]/);
  }finally{await rm(dir,{recursive:true});}
 });
}
