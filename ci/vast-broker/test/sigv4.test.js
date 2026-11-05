import test from 'node:test';
import assert from 'node:assert/strict';
import { presignGet, canonicalPath, uriEncode } from '../src/sigv4.js';
// FALSIFIER: the presigned query-string signature differs from the AWS documentation example
// ("Authenticating Requests: Using Query Parameters (AWS Signature Version 4)", GET /test.txt),
// or a changed input (secret, time, host, path, expiry) leaves the signature unchanged.
const base = {
  accessKeyId: 'AKIAIOSFODNN7EXAMPLE',
  secretAccessKey: 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY',
  host: 'examplebucket.s3.amazonaws.com',
  path: '/test.txt',
  region: 'us-east-1',
  expires: 86400,
  date: new Date('2013-05-24T00:00:00Z'),
};
const signature = (url) => new URL(url).searchParams.get('X-Amz-Signature');
test('sigv4 presign matches the AWS documented example', async () => {
  const url = await presignGet(base);
  assert.equal(
    url,
    'https://examplebucket.s3.amazonaws.com/test.txt?X-Amz-Algorithm=AWS4-HMAC-SHA256' +
      '&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request' +
      '&X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host' +
      '&X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404',
  );
});
for (const [name, patch] of Object.entries({
  secret: { secretAccessKey: 'x' + base.secretAccessKey },
  time: { date: new Date('2013-05-24T00:00:01Z') },
  day: { date: new Date('2013-05-25T00:00:00Z') },
  host: { host: 'other.s3.amazonaws.com' },
  path: { path: '/test2.txt' },
  expires: { expires: 86399 },
  region: { region: 'auto' },
}))
  test('sigv4 signature depends on ' + name, async () => {
    const changed = await presignGet({ ...base, ...patch });
    assert.notEqual(signature(changed), signature(await presignGet(base)));
  });
test('sigv4 path-style R2 URL: host, bucket path, region auto', async () => {
  const url = new URL(
    await presignGet({
      ...base,
      host: '0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com',
      path: '/kayfabe-bench-enam/specviewperf-15.0.0/original/SPECviewperf-Setup-15.0.0.exe',
      region: 'auto',
      expires: 1200,
    }),
  );
  assert.equal(url.host, '0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com');
  assert.equal(
    url.pathname,
    '/kayfabe-bench-enam/specviewperf-15.0.0/original/SPECviewperf-Setup-15.0.0.exe',
  );
  assert.equal(url.searchParams.get('X-Amz-Expires'), '1200');
  assert.match(url.searchParams.get('X-Amz-Credential'), /\/20130524\/auto\/s3\/aws4_request$/);
  assert.equal(url.searchParams.get('X-Amz-SignedHeaders'), 'host');
});
test('sigv4 encodes path segments once and keeps slashes', () => {
  assert.equal(canonicalPath('/a b/c+d/é!'), '/a%20b/c%2Bd/%C3%A9%21');
  assert.equal(uriEncode("a-b_c.d~e'()*"), 'a-b_c.d~e%27%28%29%2A');
});
for (const expires of [0, -1, 1.5, 604801, NaN, '60'])
  test('sigv4 refuses expiry ' + String(expires), async () => {
    await assert.rejects(presignGet({ ...base, expires }), RangeError);
  });
test('sigv4 refuses a bad date or a relative path', async () => {
  await assert.rejects(presignGet({ ...base, date: new Date(NaN) }), RangeError);
  await assert.rejects(presignGet({ ...base, date: '2013-05-24' }), RangeError);
  await assert.rejects(presignGet({ ...base, path: 'test.txt' }), RangeError);
});
