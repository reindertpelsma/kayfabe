import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {checkWorkflows} from '../workflow-guard.js';
const workflow=await readFile(new URL('../../../.github/workflows/gpu-vast.yml',import.meta.url),'utf8');
for(const job of ['run','utility'])test('unapproved reusable job cannot inherit endpoint secrets '+job,()=>{const errors=checkWorkflows({'gpu-vast.yml':workflow,'other.yml':'jobs:\n  '+job+':\n    uses: ./.github/workflows/reusable.yml\n    secrets: inherit\n'});assert.ok(errors.some(x=>x.startsWith('Broker endpoint')),JSON.stringify(errors));});
test('explicit unrelated reusable secret mapping remains allowed',()=>assert.deepEqual(checkWorkflows({'gpu-vast.yml':workflow,'other.yml':'jobs:\n  utility:\n    uses: ./.github/workflows/reusable.yml\n    secrets:\n      OTHER: ${{ secrets.OTHER }}\n'}),[]));
for(const alias of ['WORKER_URL','WORKER_PATH'])test('broker step name cannot reference endpoint alias '+alias,()=>{const text=workflow.replace('- name: Evaluate trusted Worker exclusions and request bounded check','- name: ${{ env.'+alias+' }}');assert.notEqual(text,workflow);const errors=checkWorkflows({'gpu-vast.yml':text});assert.ok(errors.some(x=>x.startsWith('Broker endpoint')),JSON.stringify(errors));});
