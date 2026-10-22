import test from 'node:test';
import assert from 'node:assert/strict';
import {sanitizeLog} from '../src/sanitize.js';
// FALSIFIER: a token family with a valid short payload survives because the
// generic 32-character scrubber accidentally hides missing family redaction.
for(const prefix of ['gh'+'p_','github'+'_pat_'])test('short legacy token family redaction '+prefix,()=>{
 const token=prefix+'A'.repeat(20);assert.ok(token.length<32);
 assert.equal(sanitizeLog(new TextEncoder().encode('value '+token)),'[box] value [redacted]\n');
});
for(const text of ['é'.repeat(1000),'😀'.repeat(400)])test('multibyte display line is capped by UTF8 bytes '+text.codePointAt(0),()=>{
 const result=sanitizeLog(new TextEncoder().encode(text));assert.ok(result.startsWith('[box] '));assert.ok(Buffer.byteLength(result)>0);assert.ok(Buffer.byteLength(result.trimEnd())<=1024);assert.doesNotMatch(result,/�/);
});
