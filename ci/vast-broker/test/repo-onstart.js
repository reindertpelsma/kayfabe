// Test transport for the real repo-side scripts, replacing only their env file.
// No reporter/helper logic is duplicated here.
import { readFileSync } from 'node:fs';
import { DEFAULTS } from '../src/defaults.js';
export function onstart(sha, nonce, minutes, repo, callback) {
  const environment = {
    KF_PROTOCOL: '5',
    KF_BROKER_URL: callback.url + (callback.path ? '/' + callback.path : ''),
    KF_RUN_ID: callback.run,
    KF_RUN_TOKEN: callback.token,
    KF_SHA: sha,
    KF_NONCE: nonce,
    KF_REPO: repo,
    KF_RUNNER_NONCE: callback.runnerNonce ?? '',
    KF_SOFT_DEADLINE: String(
      (callback.softDeadline ?? Date.now() + (minutes - DEFAULTS.softMarginMinutes) * 60000) / 1000,
    ),
    KF_HEARTBEAT_SECONDS: String(DEFAULTS.heartbeatMinutes * 60),
    KF_GRACE_FAIL: String((callback.graceMinutes ?? DEFAULTS.failGraceMinutes) * 60),
  };
  environment.KF_SOFT_DEADLINE = String(Math.floor(Number(environment.KF_SOFT_DEADLINE)));
  const exports = Object.entries(environment)
    .map(([key, value]) => `export ${key}='${value}'`)
    .join('\n');
  const shell = readFileSync(
    new URL('../../../scripts/ci/vast_onstart.sh', import.meta.url),
    'utf8',
  ).replaceAll('. /run/kf-ci/env', exports);
  const python = readFileSync(
    new URL('../../../scripts/ci/vast_report.py', import.meta.url),
    'utf8',
  );
  return (
    shell +
    "\npython3 - <<'KF_REPORT'\nimport os\nos.environ.update(" +
    JSON.stringify(environment) +
    ')\n' +
    python +
    '\nKF_REPORT\n'
  );
}
