import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {checkWorkflows} from '../workflow-guard.js';
const actual=await readFile(new URL('../../../.github/workflows/gpu-vast.yml',import.meta.url),'utf8');
const bindings='          WORKER_URL: ${{ secrets.VAST_BROKER_URL }}\n          WORKER_PATH: ${{ secrets.VAST_BROKER_PATH }}\n';
const canonical=actual.replace('      WORKER_URL: ${{ vars.VAST_BROKER_URL }}\n','').replace('        id: broker\n        shell: bash\n        env:\n','        id: broker\n        shell: bash\n        env:\n'+bindings);
const safe=actual.includes('secrets.VAST_BROKER_URL')?actual:canonical;
const errors=(text,extra={})=>checkWorkflows({'gpu-vast.yml':text,...extra});
const rejected=(text,extra)=>assert.ok(errors(text,extra).some(x=>x.startsWith('Broker endpoint')),JSON.stringify(errors(text,extra)));
test('actual broker uses Actions secrets only inside its execution step',()=>{assert.doesNotMatch(actual,/vars\.VAST_BROKER/);assert.match(actual,/          WORKER_URL: \$\{\{ secrets\.VAST_BROKER_URL \}\}/);assert.match(actual,/          WORKER_PATH: \$\{\{ secrets\.VAST_BROKER_PATH \}\}/);const job=actual.split('  gpu:\n')[1].split('  report:\n')[0];assert.doesNotMatch(job.split('    steps:')[0],/VAST_BROKER_URL|VAST_BROKER_PATH/);});
test('canonical broker step secret bindings pass the guard',()=>assert.deepEqual(errors(safe),[]));
for(const name of ['URL','PATH']){
 test('public variable cannot replace broker '+name+' secret',()=>rejected(safe.replace('secrets.VAST_BROKER_'+name,'vars.VAST_BROKER_'+name)));
 for(const id of ['pending','report','run'])test('another job cannot receive '+name+' secret '+id,()=>rejected(safe+'  '+(id==='run'?'run':'leak-'+id)+':\n    env:\n      ENDPOINT: ${{ secrets.VAST_BROKER_'+name+' }}\n    steps: []\n'));
 for(const sink of ['name','run','summary'])test('secret context cannot enter '+sink+' '+name,()=>{const line=sink==='name'?'        name: ${{ secrets.VAST_BROKER_'+name+' }}':sink==='run'?'          echo "${{ secrets.VAST_BROKER_'+name+' }}"':'          printf "%s" "${{ secrets.VAST_BROKER_'+name+' }}" >> "$GITHUB_STEP_SUMMARY"';rejected(safe.replace('          set -euo pipefail',line+'\n          set -euo pipefail'));});
 test('other workflow cannot receive '+name+' secret',()=>rejected(safe,{'other.yml':'jobs:\n  work:\n    env:\n      HIDDEN: ${{ secrets.VAST_BROKER_'+name+' }}\n    steps: []\n'}));
 test('secret binding in a second broker step fails '+name,()=>rejected(safe.replace('  report:\n','      - name: Other\n        env:\n          WORKER_'+(name==='URL'?'URL':'PATH')+': ${{ secrets.VAST_BROKER_'+name+' }}\n        run: true\n  report:\n')));
}
for(const expression of ["toJSON(secrets)","secrets[inputs.name]","secrets['VAST_BROKER_URL']"])test('opaque secret object access fails '+expression,()=>rejected(safe+'  extra:\n    env:\n      ALL: ${{ '+expression+' }}\n    steps: []\n'));
for(const alias of ['WORKER_URL','WORKER_PATH']){
 test('endpoint alias cannot enter another job '+alias,()=>rejected(safe+'  extra:\n    env:\n      '+alias+': hidden\n    steps: []\n'));
 test('broker cannot echo endpoint alias '+alias,()=>rejected(safe.replace('          set -euo pipefail','          echo "$'+alias+'"\n          set -euo pipefail')));
 test('broker cannot put endpoint alias in a summary '+alias,()=>rejected(safe.replace('          set -euo pipefail','          printf "%s" "$'+alias+'" >> "$GITHUB_STEP_SUMMARY"\n          set -euo pipefail')));
 test('broker cannot export endpoint alias '+alias,()=>rejected(safe.replace('          set -euo pipefail','          printf "url=%s" "$'+alias+'" >> "$GITHUB_OUTPUT"\n          set -euo pipefail')));
}
for(const output of ['url','path'])test('broker job output cannot carry endpoint '+output,()=>rejected(safe.replace('    outputs:\n','    outputs:\n      '+output+': ${{ steps.broker.outputs.'+output+' }}\n')));
for(const name of ['WORKER_URL','BROKER_PATH_PREFIX'])test('Worker endpoint binding is a secret '+name,async()=>{const config=await readFile(new URL('../wrangler.toml',import.meta.url),'utf8');assert.doesNotMatch(config,new RegExp('^'+name+'\\s*=','m'));assert.match(config,new RegExp(name));});
