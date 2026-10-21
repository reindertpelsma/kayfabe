import { beforeAll, beforeEach, expect, it, vi } from 'vitest';
let mod: any, pair: CryptoKeyPair, other: CryptoKeyPair, jwk: JsonWebKey;
const stamp = 1800000000;
const valid = {iss: 'https://token.actions.githubusercontent.com', aud: 'https://broker.example',
  exp: stamp + 300, nbf: stamp - 1, repository: 'reindertpelsma/kayfabe',
  event_name: 'merge_group', ref: 'refs/heads/gh-readonly-queue/master/pr-1', run_id: '123', run_attempt: '1'};
const encode = (v: any) => Buffer.from(JSON.stringify(v)).toString('base64url');
async function token(patch = {}, header = {}, key = pair.privateKey) {
  const input = encode({alg: 'RS256', kid: 'one', ...header}) + '.' + encode({...valid, ...patch});
  const sig = await crypto.subtle.sign('RSASSA-PKCS1-v1_5', key, new TextEncoder().encode(input));
  return input + '.' + Buffer.from(sig).toString('base64url');
}
beforeAll(async () => {
  const alg = {name: 'RSASSA-PKCS1-v1_5', modulusLength: 2048, publicExponent: new Uint8Array([1,0,1]), hash: 'SHA-256'};
  pair = await crypto.subtle.generateKey(alg, true, ['sign', 'verify']);
  other = await crypto.subtle.generateKey(alg, true, ['sign', 'verify']);
  jwk = await crypto.subtle.exportKey('jwk', pair.publicKey);
});
let verifier: any, fetcher: any;
beforeEach(async () => {
  mod = await import('../src/oidc');
  fetcher = vi.fn(async () => Response.json({keys: [{...jwk, kid: 'one', alg: 'RS256', use: 'sig'}]}));
  verifier = new mod.OidcVerifier(fetcher, () => stamp * 1000);
});
it('verifies a signed merge-group identity', async () => {
  expect(await verifier.verify(await token(), valid.aud)).toMatchObject({run_id: '123', run_attempt: '1'});
  expect(fetcher.mock.calls[0][0]).toBe('https://token.actions.githubusercontent.com/.well-known/jwks');
});
it('accepts manual dispatch on master', async () => {
  expect(await verifier.verify(await token({event_name: 'workflow_dispatch', ref: 'refs/heads/master'}), valid.aud)).toBeTruthy();
});
it.each([
  {iss: 'https://evil.example'}, {aud: 'https://wrong.example'}, {aud: [valid.aud]},
  {repository: 'elsewhere/kayfabe'}, {event_name: 'pull_request'}, {event_name: 'push'},
  {event_name: 'workflow_dispatch', ref: 'refs/heads/feature'},
  {exp: stamp}, {exp: undefined}, {exp: 'later'}, {nbf: stamp + 1}, {nbf: undefined},
  {run_id: undefined}, {run_id: '../123'}, {run_attempt: '0'}, {run_attempt: undefined},
])('rejects unauthorized or missing claims %j', async patch => {
  await expect(verifier.verify(await token(patch), valid.aud)).rejects.toThrow();
});
it('rejects an otherwise valid token with a forged signature', async () => {
  await expect(verifier.verify(await token({}, {}, other.privateKey), valid.aud)).rejects.toThrow();
});
it.each([{alg: 'none'}, {alg: 'HS256'}, {kid: undefined}, {jku: 'https://evil.example'}])('rejects attacker-controlled header %j', async header => {
  await expect(verifier.verify(await token({}, header), valid.aud)).rejects.toThrow();
});
it('caches JWKS for repeated verification', async () => {
  const t = await token(); await verifier.verify(t, valid.aud); await verifier.verify(t, valid.aud); expect(fetcher).toHaveBeenCalledTimes(1);
});
it('refreshes JWKS on an unknown rotated key', async () => {
  await verifier.verify(await token(), valid.aud);
  const second = await crypto.subtle.exportKey('jwk', other.publicKey);
  fetcher.mockImplementation(async () => Response.json({keys: [{...second, kid: 'two', alg: 'RS256', use: 'sig'}]}));
  expect(await verifier.verify(await token({}, {kid: 'two'}, other.privateKey), valid.aud)).toBeTruthy();
  expect(fetcher).toHaveBeenCalledTimes(2);
});
it('refreshes on same-kid rotation and still verifies the signature', async () => {
  await verifier.verify(await token(), valid.aud);
  const second = await crypto.subtle.exportKey('jwk', other.publicKey);
  fetcher.mockImplementation(async () => Response.json({keys: [{...second, kid: 'one', alg: 'RS256', use: 'sig'}]}));
  expect(await verifier.verify(await token({}, {}, other.privateKey), valid.aud)).toBeTruthy();
});
it('does not echo a failed JWKS body', async () => {
  fetcher.mockImplementation(async () => new Response('SENSITIVE', {status: 500}));
  await expect(verifier.verify(await token(), valid.aud)).rejects.not.toThrow('SENSITIVE');
});
