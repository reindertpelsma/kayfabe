// prettier-ignore
import {refuse} from './policy.js';
export function checkedResult(result) {
  const object = (x) => x !== null && typeof x === 'object' && !Array.isArray(x);
  if (
    !object(result) ||
    Object.keys(result).sort().join(',') !== 'hardware,nonce,runner_nonce,sha,suites,verdict' ||
    !['PASS', 'FAIL', 'UNAVAILABLE'].includes(result.verdict) ||
    !/^([0-9a-f]{32})$/.test(result.nonce) ||
    !/^([0-9a-f]{64})$/.test(result.runner_nonce) ||
    !/^([0-9a-f]{40})$/.test(result.sha)
  )
    refuse('BAD_REPORT', 400);
  const rows = result.suites;
  if (
    !Array.isArray(rows) ||
    rows.length < 1 ||
    rows.length > 20 ||
    new Set(rows.map((x) => x?.name)).size !== rows.length
  )
    refuse('BAD_REPORT', 400);
  for (const r of rows)
    if (
      !object(r) ||
      Object.keys(r).sort().join(',') !== 'fail,name,pass,skip' ||
      typeof r.name !== 'string' ||
      !/^[a-z][a-z0-9_]{0,47}$/.test(r.name) ||
      !['pass', 'fail', 'skip'].every(
        (k) => Number.isSafeInteger(r[k]) && r[k] >= 0 && r[k] <= 10000000,
      )
    )
      refuse('BAD_REPORT', 400);
  const h = result.hardware;
  if (
    !object(h) ||
    Object.keys(h).sort().join(',') !== 'compute_cap,driver_version,gpu_name' ||
    typeof h.gpu_name !== 'string' ||
    !/^[A-Za-z0-9_. -]{1,100}$/.test(h.gpu_name) ||
    !Number.isSafeInteger(h.compute_cap) ||
    h.compute_cap < 0 ||
    h.compute_cap > 2000 ||
    typeof h.driver_version !== 'string' ||
    !/^(?:unknown|\d{3}\.\d{1,3}(?:\.\d{1,2})?)$/.test(h.driver_version)
  )
    refuse('BAD_REPORT', 400);
  if (result.verdict === 'PASS') {
    if (
      rows.some((r) => r.fail !== 0 || r.skip !== 0 || r.pass <= 0) ||
      h.gpu_name === 'unknown' ||
      h.driver_version === 'unknown' ||
      h.compute_cap < 750
    )
      refuse('BAD_REPORT', 400);
  } else if (!rows.some((r) => r.fail > 0)) refuse('BAD_REPORT', 400);
  return result;
}
