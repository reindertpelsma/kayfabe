import {
  cpSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  rmSync,
  symlinkSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const broker = fileURLToPath(new URL('.', import.meta.url)),
  repo = fileURLToPath(new URL('../..', import.meta.url));
const scratch = mkdtempSync(join(tmpdir(), 'kf-J2-provider-'));
const cases = [
  {
    name: 'review interleaved recovery reuses the old fatal timer',
    file: 'src/provider-policy.js',
    before: 'outcome.progress && reason',
    after: 'false && reason',
    pattern: '^RV2 interleaved progress restarts current fatal confirmation',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review JS signed port operand is not consumed',
    file: 'src/rental.js',
    before: "if (text[end] === '+' || text[end] === '-') end++;",
    after: 'if (false) end++;',
    pattern: '^RV2 structural port spellings JS property',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review runner signed port operand is not consumed',
    file: 'runner.py',
    before: 'if end < len(text) and text[end] in "+-":',
    after: 'if False:',
    pattern: '^RV2 structural port spellings Python property',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review renderer signed port operand is not consumed',
    file: 'public-log.js',
    before: "if end < len(text) and text[end] in '+-':",
    after: 'if False:',
    pattern: '^RV2 structural port spellings Python property',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review boot batch is scanned in reverse order',
    file: 'src/provider-policy.js',
    before: "for (const line of text.split('\\n'))",
    after: "for (const line of text.split('\\n').reverse())",
    pattern: '^RV2 ordered boot batch fatal-progress',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review later progress does not clear fatal evidence',
    file: 'src/provider-policy.js',
    before: '      reason = null;\n      progress = true;',
    after: '      progress = true;',
    pattern: '^RV2 ordered boot batch fatal-progress',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review JS numeric port operand is width limited',
    file: 'src/rental.js',
    before: "while (end < text.length && text[end] >= '0' && text[end] <= '9')",
    after: "while (end - digits < 5 && end < text.length && text[end] >= '0' && text[end] <= '9')",
    pattern: '^RV2 structural port spellings JS property',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review runner numeric port operand is width limited',
    file: 'runner.py',
    before: 'while end < len(text) and text[end] in "0123456789":',
    after: 'while end - digits < 5 and end < len(text) and text[end] in "0123456789":',
    pattern: '^RV2 structural port spellings Python property',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review renderer numeric port operand is width limited',
    file: 'public-log.js',
    before: "while end < len(text) and text[end] in '0123456789':",
    after: "while end - digits < 5 and end < len(text) and text[end] in '0123456789':",
    pattern: '^RV2 structural port spellings Python property',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review recovered status text becomes current on a flap',
    file: 'src/provider-policy.js',
    before: "historicalStatus ? null : fatalSignature(instance, '', id, policy)",
    after: "fatalSignature(instance, '', id, policy)",
    pattern: '^RV2 healthy status text stays historical across provider flap',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review recovery suppresses a newly changed fatal status',
    file: 'src/provider-policy.js',
    before: 'const historicalStatus = cursor?.healthyStatusHash === statusHash;',
    after: 'const historicalStatus = Boolean(cursor?.healthyStatusHash);',
    pattern: '^RV2 changed fatal status after healthy baseline remains actionable',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review healthy state does not establish log baseline',
    file: 'src/provider-policy.js',
    before: 'cursor: recoveryCursor',
    after: 'cursor',
    pattern: '^RV2 terminal historical log after running remains inert',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review unreadable recovery trusts old log continuity',
    file: 'src/provider-policy.js',
    before: "typeof log === 'string' ? delta.cursor : {}",
    after: "typeof log === 'string' ? delta.cursor : (cursor ?? {})",
    pattern: '^RV2 unreadable recovery baseline fences historical fatal log',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review healthy baseline suppresses newly emitted fatal text',
    file: 'src/provider-policy.js',
    before: 'let reason = outcome.reason;',
    after: 'let reason = cursor ? null : outcome.reason;',
    pattern: '^RV2 fresh fatal after healthy baseline still confirms within bound',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review unknown provider state earns hardware blame',
    file: 'src/provider-policy.js',
    before: '!BOOT_FAILURE_STATES.has(state)',
    after: "state === 'running'",
    pattern: '^RV2 malformed provider state cannot confirm hardware blame',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review fatal elapsed window bypassed',
    file: 'src/provider-policy.js',
    before: 'at - next.first >= policy.confirmationSeconds * 1000',
    after: 'true',
    pattern: '^RV2 fatal helper enforces elapsed window',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review close polls count as independent observations',
    file: 'src/provider-policy.js',
    before: 'policy.observationSpacingSeconds * 1000',
    after: '0',
    pattern: '^RV2 close polls are not independent fatal observations',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review retained boot log is rescanned',
    file: 'src/provider-policy.js',
    before: 'orderedBootFailure(delta.fresh, id, policy)',
    after: 'orderedBootFailure(log, id, policy)',
    pattern: '^RV2 terminal historical log after running remains inert',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review running state no longer clears fatal status',
    file: 'src/provider-policy.js',
    before: 'return { reason: null, cursor: recoveryCursor };',
    after: "return { reason: fatalSignature(instance, '', id, policy), cursor: recoveryCursor };",
    pattern: '^RV2 running state cancels a retained fatal status message',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review unchanged current failure loses its witness',
    file: 'src/provider-policy.js',
    before: "prior?.source === 'boot_log' &&\n    prior.state === state &&",
    after: "prior?.source === 'boot_log' &&\n    false &&",
    pattern: '^RV2 unchanged current fatal waits for window',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review changed provider progress preserves old witness',
    file: 'src/provider-policy.js',
    before: 'prior.statusHash === statusHash\n  ) {',
    after: 'true\n  ) {',
    pattern: '^RV2 changing provider progress clears prior witness',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review boot cursor history is unbounded',
    file: 'src/provider-policy.js',
    before: 'lines.slice(-64)',
    after: 'lines.slice(0)',
    pattern: '^RV2 log cursor storage stays bounded',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review observation spacing ignores configured data',
    file: 'src/provider-policy.js',
    before: 'policy.observationSpacingSeconds * 1000',
    after: '1000',
    pattern: '^RV2 fatal observation spacing comes from validated data',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review old confirmed history blames a running timeout',
    file: 'src/index.js',
    before: "['created', 'loading', 'starting', 'offline', 'null']",
    after:
      "(record.bootFailure?.confirmed ? ['running'] : ['created', 'loading', 'starting', 'offline', 'null'])",
    pattern: '^RV2 running boot timeout ignores old confirmed history',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review compact Port remains in JS output',
    file: 'src/rental.js',
    before: 'text = redactPortFields(text);',
    after: '/* test mutation: structural fields retained */',
    pattern: '^RV2 compact Port removed by JS boundaries',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review compact Port remains in runner output',
    file: 'runner.py',
    before: 'text = redact_port_fields(text)',
    after: 'pass',
    pattern: '^RV2 compact Port removed by Python pipelines',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review compact Port remains in renderer output',
    file: 'public-log.js',
    before: 'text = redact_port_fields(text)',
    after: 'pass',
    pattern: '^RV2 compact Port removed by Python pipelines',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review publication drops later attempts',
    file: 'src/publish.js',
    before: '((?:[0-9]|[12][0-9]|3[01]))',
    after: '([0-9])',
    pattern: '^RV2 settled later attempt publishes log and structured report',
    test: 'test/run-report-extra.test.js',
    all: false,
  },
  {
    name: 'review publication accepts mismatched attempt identity',
    file: 'src/publish.js',
    before: 'entry.leg !== Number(m[2])',
    after: 'false',
    pattern: '^RV2 publication rejects mismatched later attempt identity',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review publication accepts unsupported attempt range',
    file: 'src/publish.js',
    before: '[12][0-9]|3[01]',
    after: '[123][0-9]',
    pattern: '^RV2 publication rejects malformed attempt',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review penalty clock advances within one market',
    file: 'src/offers.js',
    before: 'evaluatedAt,',
    after: 'this.now(),',
    pattern: '^RV2 advancing clock preserves equal penalty ranking',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'review first report marker is not persisted',
    file: 'src/index.js',
    before: 'r.firstReportAt ??= now();',
    after: '/* test mutation: marker omitted */',
    pattern: '^RV2 first accepted log permanently fences provider replacement',
    test: 'test/provider-review.test.js',
    all: true,
  },
  {
    name: 'review settled failures keep full TTL liability',
    file: 'src/ledger.js',
    before: 'v.ended === null ? (Math.max(0, v.started + v.ttl - now) * v.price) / 3600000 : 0',
    after: '(Math.max(0, v.started + v.ttl - now) * v.price) / 3600000',
    pattern: '^RV2 settled failures release unused TTL reservations',
    test: 'test/provider-review.test.js',
    all: false,
  },
  {
    name: 'daily room replaces the original total-run cap',
    file: 'src/index.js',
    before: 'profileCaps(cfg, profile).price',
    after: 'profileCaps(cfg, profile).dailyUsd',
    all: false,
    pattern: '^session cumulative transfers obey original total-run ceiling',
    test: 'test/provider-session.test.js',
  },
  {
    name: 'provider stop before reporting becomes a test failure',
    file: 'src/index.js',
    before: "record.firstReportAt === undefined ? 'PROVIDER_FAULT' : 'ENDED'",
    after: "'ENDED'",
    all: false,
    pattern: '^worst provider stops before first report',
    test: 'test/provider-worst.test.js',
  },
  {
    name: 'same-label foreign contract gains rental authority',
    file: 'src/vast.js',
    before: '(await sha256Hex(i.onstart)) !== record.scriptHash',
    after: 'false',
    all: false,
    pattern: '^rules same-label foreign contract has no rental authority',
  },
  {
    name: 'compact SSH port is exposed',
    file: 'src/rental.js',
    before: '|-[pP]|',
    after: '|-q|',
    all: false,
    pattern: '^rules compact SSH port remains private',
  },
  {
    name: 'middle pin ignores remaining spending room',
    file: 'src/index.js',
    before: 'evaluation.allowed = affordable;',
    after: ';',
    all: false,
    pattern: '^session pinned fallback respects remaining spending room',
    test: 'test/provider-session.test.js',
  },
  {
    name: 'later evidence attempt is not discovered',
    file: 'src/ingest.js',
    before: '(?:[0-9]|[12][0-9]|3[01])',
    after: '[0-9]',
    all: false,
    pattern: '^rules later attempt tree queue remains bounded',
  },
  {
    name: 'out of range evidence attempt is discovered',
    file: 'src/ingest.js',
    before: '(?:[0-9]|[12][0-9]|3[01])',
    after: '[0-9]+',
    all: false,
    pattern: '^rules later attempt tree queue remains bounded',
  },
  {
    name: 'pre-report provider loss stops replacement',
    file: 'src/provider-policy.js',
    before: "return record.firstReportAt === undefined ? 'PROVIDER_FAULT' : 'FAIL';",
    after: "return 'UNAVAILABLE';",
    all: false,
    pattern: '^worst pre-report',
    test: 'test/provider-worst.test.js',
  },
  {
    name: 'unattributed provision timeout penalises hardware',
    file: 'src/index.js',
    before: 'if (attributable)',
    after: 'if (true)',
    all: false,
    pattern: '^worst running without first report can replace without hardware blame',
    test: 'test/provider-worst.test.js',
  },
  {
    name: 'pre-report provision timeout stops replacement',
    file: 'src/index.js',
    before: "reason === 'BOOT_TIMEOUT' && record.firstReportAt === undefined;",
    after: 'false;',
    all: false,
    pattern: '^worst running without first report can replace without hardware blame',
    test: 'test/provider-worst.test.js',
  },
  {
    name: 'final failure skips log proof',
    file: 'runner.py',
    before: '                    and data["result_at"] is None\n',
    after: '',
    all: false,
    pattern: '^runner provider failed-hash',
    test: 'test/provider-runner.test.js',
  },
  {
    name: 'configured attempt limit ignored',
    file: 'src/provider-policy.js',
    before: '< policy.maxAttempts',
    after: '< 8',
    all: false,
    pattern: '^session attempt cap',
    test: 'test/provider-session.test.js',
  },
  {
    name: 'reported record learns hardware penalty',
    file: 'src/ledger.js',
    before: 'if (record.firstReportAt !== undefined) return;',
    after: '',
    all: false,
    pattern: '^rules fault escalation decay and cap',
  },
  {
    name: 'permanent hardware controls migrated away',
    file: 'src/ledger.js',
    before: 'value?.permanent === true ||',
    after: '',
    all: false,
    pattern: '^rules legacy fault migration preserves permanent controls',
  },
  {
    name: 'duplicate success clears penalty again',
    file: 'src/ledger.js',
    before: '!history || history.success === record.nonce',
    after: '!history',
    all: false,
    pattern: '^rules fault escalation decay and cap',
  },
  {
    name: 'endpoint mask shreds non-address facts',
    file: 'src/rental.js',
    before: "prefix === 'unknown' ? value : prefix",
    after: 'prefix',
    all: false,
    pattern: '^rules address masking preserves non-address facts',
  },
  {
    name: 'persisted reported fault returns retry',
    file: 'src/ingest.js',
    before: "['FAIL', 'PROVIDER_FAULT', 'UNAVAILABLE'].includes(r.verdict)",
    after: "r.verdict === 'FAIL'",
    all: false,
    pattern: '^rules persisted reported fault is stored as failure',
  },
  {
    name: 'generic create refusal penalises hardware',
    file: 'src/index.js',
    before: 'r.bandwidthCost = 0;\n        await closeLine',
    after:
      'r.bandwidthCost = 0;\n        await ledger.markProviderFault(r, r.faultReason, now(), cfg.provider);\n        await closeLine',
    all: false,
    pattern: '^rules generic create refusal never penalises hardware',
  },
  {
    name: 'runner spins during a temporary market gap',
    file: 'runner.py',
    before: 'if marker == self.last_requested_attempt:',
    after: 'if False:',
    all: false,
    pattern: '^runner provider waiting',
    test: 'test/provider-runner.test.js',
  },
  {
    name: 'uncreated replacement skips daily count',
    file: 'src/ledger.js',
    before: 'summary.counted_attempt ||',
    after: 'true ||',
    all: false,
    pattern: '^rules uncreated retry needs a daily count slot',
  },
  {
    name: 'temporary market gap exhausts session',
    file: 'src/index.js',
    before: 'if (!admitted && retrySource) await waitReplacement(retrySource, cfg);',
    after: 'if (!admitted && retrySource) await exhaust(retrySource, cfg);',
    all: false,
    pattern: '^session waits for fresh market within original deadline',
    test: 'test/provider-session.test.js',
  },
  {
    name: 'fatal confirmation disabled',
    file: 'src/provider-policy.js',
    before: 'next.count >= 2',
    after: 'next.count >= 200',
    all: false,
    pattern: '^rules fatal confirmation',
  },
  {
    name: 'unknown provider errors treated as fatal',
    file: 'src/provider-policy.js',
    before: '  return null;\n}\nexport function providerMessageClass',
    after: "  return 'GPU_START_ERROR';\n}\nexport function providerMessageClass",
    all: false,
    pattern: '^rules KVM exact owned id',
  },
  {
    name: 'first report retry guard disabled',
    file: 'src/provider-policy.js',
    before: '    record.firstReportAt === undefined &&',
    after: '    true &&',
    all: false,
    pattern: '^rules bounded attempt and wall clock',
  },
  {
    name: 'reported provider result remains retryable',
    file: 'src/index.js',
    before: "if (['PROVIDER_FAULT', 'UNAVAILABLE'].includes(r.result.verdict))",
    after: 'if (false)',
    all: false,
    pattern: '^rules report failure cannot be rerented',
  },
  {
    name: 'maximum attempt guard disabled',
    file: 'src/provider-policy.js',
    before: '(record.providerRetries ?? 0) + 1 < policy.maxAttempts',
    after: 'true',
    all: false,
    pattern: '^rules bounded attempt and wall clock',
  },
  {
    name: 'session deadline guard disabled',
    file: 'src/provider-policy.js',
    before:
      'at <\n      (record.sessionDeadline ??\n        (record.originalStarted ?? record.started) + policy.sessionMinutes * 60000)',
    after: 'true',
    all: false,
    pattern: '^rules bounded attempt and wall clock',
  },
  {
    name: 'cleanup proof disabled',
    file: 'src/provider-policy.js',
    before: 'Number.isFinite(record.ended)',
    after: 'true',
    all: false,
    pattern: '^rules pending cleanup cannot authorise retry',
  },
  {
    name: 'absence verification disabled',
    file: 'src/vast.js',
    before: 'if ((await this.inspect(record)) !== null) refuse();',
    after: 'void record;',
    all: false,
    pattern: '^rules teardown needs verified absence',
  },
  {
    name: 'initial foreign contract admitted',
    file: 'src/vast.js',
    before: 'record.proofVersion === 1 && !record.contractVerified',
    after: 'false',
    all: false,
    pattern: '^rules foreign initial contract is never accepted',
  },
  {
    name: 'run cost cap disabled',
    file: 'src/ledger.js',
    before: 'spent + cost > prior.runBudgetUsd',
    after: 'false',
    all: false,
    pattern: '^rules cumulative cap is transactional',
  },
  {
    name: 'daily cost cap disabled',
    file: 'src/ledger.js',
    before: 'if (values.daily_usd + cost > values.daily_cap_usd)',
    after: 'if (false)',
    all: false,
    pattern: '^rules daily cap cannot be bypassed by replacement',
  },
  {
    name: 'spend receipt archive lost',
    file: 'src/ledger.js',
    before:
      "await ledger.put('cost:' + record.key + ':provider:' + (prior.providerRetries ?? 0), {",
    after:
      "await ledger.put('cost:' + record.key + ':discarded:' + (prior.providerRetries ?? 0), {",
    all: false,
    pattern: '^rules reservation archives every attempt',
  },
  {
    name: 'previous failed machine admitted',
    file: 'src/ledger.js',
    before: 'record.machineId === prior.machineId',
    after: 'false',
    all: false,
    pattern: '^rules previous machine is forbidden',
  },
  {
    name: 'previous failed host admitted',
    file: 'src/ledger.js',
    before: 'record.hostId === prior.hostId',
    after: 'false',
    all: false,
    pattern: '^rules previous host is forbidden',
  },
  {
    name: 'prior attempt receipts dropped',
    file: 'src/ledger.js',
    before:
      'JSON.stringify(record.rentalAttempts ?? []) !==\n            JSON.stringify(prior.rentalAttempts ?? [])',
    after: 'false',
    all: false,
    pattern: '^rules prior attempt receipts cannot disappear',
  },
  {
    name: 'ranking ignores penalties',
    file: 'src/provider-policy.js',
    before: 'offer.pricing.effective + (offer.penalty ?? 0)',
    after: 'offer.pricing.effective',
    all: false,
    pattern: '^rules middle pin order and penalty',
  },
  {
    name: 'penalty decay disabled',
    file: 'src/provider-policy.js',
    before: 'policy.penaltyHalfLifeHours * 3600000',
    after: 'Infinity',
    all: false,
    pattern: '^rules fault escalation decay and cap',
  },
  {
    name: 'penalty cap disabled',
    file: 'src/provider-policy.js',
    before: 'return Math.min(policy.penaltyCapUsdHour, sum);',
    after: 'return sum;',
    all: false,
    pattern: '^rules fault escalation decay and cap',
  },
  {
    name: 'penalty escalation disabled',
    file: 'src/ledger.js',
    before: '2 ** Math.min(prior, policy.historyPerIdentity)',
    after: '1',
    all: false,
    pattern: '^rules fault escalation decay and cap',
  },
  {
    name: 'success wipes penalty immediately',
    file: 'src/ledger.js',
    before: 'f.weight * policy.successFactor',
    after: '0',
    all: false,
    pattern: '^rules fault escalation decay and cap',
  },
  {
    name: 'fault history per identity unbounded',
    file: 'src/ledger.js',
    before: '-policy.historyPerIdentity,',
    after: '0,',
    all: false,
    pattern: '^rules fault escalation decay and cap',
  },
  {
    name: 'fault identity storage unbounded',
    file: 'src/ledger.js',
    before: 'await storage.delete(oldest[0]);',
    after: 'void oldest;',
    all: true,
    pattern: '^rules penalty history storage bound',
  },
  {
    name: 'ranking loses deterministic tie break',
    file: 'src/offers.js',
    before: 'a.id - b.id',
    after: '0',
    all: true,
    pattern: '^rules all penalised still rentable and deterministic',
  },
  {
    name: 'IPv4 prefix masking disabled',
    file: 'src/rental.js',
    before: "return parts.slice(0, 3).join('.') + '.0';",
    after: 'return value;',
    all: false,
    pattern: '^rules no endpoint in public text',
  },
  {
    name: 'IPv6 prefix masking disabled',
    file: 'src/rental.js',
    before: 'export function ipPrefix(value) {',
    after:
      "export function ipPrefix(value) {\n  if(typeof value==='string' && value.includes(':'))return value;",
    all: false,
    pattern: '^rules no endpoint in public text',
  },
  {
    name: 'ssh port masking disabled',
    file: 'src/rental.js',
    before: '(?:ssh[_ -]?(?:port|host)|port)',
    after: '(?:NEVER_SSH_PORT)',
    all: false,
    pattern: '^rules no endpoint in public text',
  },
  {
    name: 'middle pin bypassed',
    file: 'src/offers.js',
    before: '&& cfg.pinnedMiddleDies.length)',
    after: '&& false)',
    all: false,
    pattern: '^rules middle pin order and penalty',
  },
  {
    name: 'exhaustion omits attempted identities',
    file: 'src/index.js',
    before: 'for (const a of r.rentalAttempts ?? [])',
    after: 'for (const a of [])',
    all: false,
    pattern: '^rules exhaustion lists every attempt',
  },

  {
    name: 'public renderer endpoint masking disabled',
    file: 'public-log.js',
    before: 'def redact_endpoints(text):',
    after: 'def redact_endpoints(text):\n    return text',
    all: false,
    pattern: '^public renderer masks',
    test: 'test/provider-public.test.js',
  },
  {
    name: 'runner endpoint masking disabled',
    file: 'runner.py',
    before: 'def redact_endpoints(text):',
    after: 'def redact_endpoints(text):\n    return text',
    all: false,
    pattern: '^runner provider (pass|fail)',
    test: 'test/provider-runner.test.js',
  },
  {
    name: 'runner retries after a report',
    file: 'runner.py',
    before: 'if data.get("last_post_at") is not None or data.get("result_at") is not None:',
    after: 'if False:',
    all: false,
    pattern: '^runner provider reported',
    test: 'test/provider-runner.test.js',
  },
  {
    name: 'runner downgrades reported outage',
    file: 'runner.py',
    before:
      'self.metadata\n                and self.metadata.get("last_post_at") is not None\n                and not (result["verified"] == 1 and result["verdict"] == "PASS")',
    after: 'False',
    all: false,
    pattern: '^runner provider outage',
    test: 'test/provider-runner.test.js',
  },

  {
    name: 'middle pin eligibility expansion omitted',
    file: 'src/policy.js',
    before: '...cfg.pinnedMiddleDies',
    after: '...[]',
    all: false,
    pattern: '^rules middle pin order and penalty',
  },
  {
    name: 'renamed owned contract loses startup proof',
    file: 'src/vast.js',
    before: "typeof i.onstart !== 'string' ||",
    after: 'true ||',
    all: false,
    pattern: '^rules renamed owned contract has unique startup proof',
  },
];
function run(pattern, filename = 'test/provider-rules.test.js') {
  const r = spawnSync(
    process.execPath,
    [
      '--test',
      '--test-reporter=tap',
      ...(pattern ? ['--test-name-pattern', pattern] : []),
      filename,
    ],
    {
      cwd: join(scratch, 'ci/vast-broker'),
      env: Object.fromEntries(
        Object.entries(process.env).filter(([key]) => key !== 'NODE_TEST_CONTEXT'),
      ),
      encoding: 'utf8',
      timeout: 120000,
      maxBuffer: 8 * 1024 * 1024,
    },
  );
  if (r.error) throw r.error;
  const failures = [];
  for (const block of r.stdout.split(/^# Subtest: /m)) {
    const name = block.match(/^not ok \d+ - (.+)$/m)?.[1];
    if (!name) continue;
    if (!/code: 'ERR_ASSERTION'/.test(block)) throw Error('non-assertion failure: ' + name);
    failures.push(name);
  }
  if (r.status !== 0 && !failures.length) throw Error(r.stdout + r.stderr);
  const count = Number(r.stdout.match(/^# tests (\d+)$/m)?.[1]);
  if (!Number.isSafeInteger(count) || count < 2) throw Error('missing TAP coverage');
  return {
    status: r.status,
    tests: count,
    failures: [...new Set(failures)],
  };
}
try {
  cpSync(broker, join(scratch, 'ci/vast-broker'), {
    recursive: true,
    filter: (p) => !p.includes('/node_modules') && !p.includes('/.wrangler'),
  });
  symlinkSync(join(broker, 'node_modules'), join(scratch, 'ci/vast-broker/node_modules'), 'dir');
  for (const path of [
    'scripts/ci/vast_report.py',
    'scripts/ci/vast_payload.py',
    'scripts/ci/release_driver_pins.py',
    'ci/vast-reaper',
    'ci/platform-api/migrations',
    'ci/requirements.toml',
  ]) {
    mkdirSync(dirname(join(scratch, path)), { recursive: true });
    cpSync(join(repo, path), join(scratch, path), { recursive: true });
  }
  const baseline = run();
  if (baseline.status !== 0) throw Error('baseline failed: ' + JSON.stringify(baseline));
  const reviewBaseline = run(undefined, 'test/provider-review.test.js');
  if (reviewBaseline.status !== 0)
    throw Error('review baseline failed: ' + JSON.stringify(reviewBaseline));
  const publicationBaseline = run('^RV2 settled later', 'test/run-report-extra.test.js');
  if (publicationBaseline.status !== 0)
    throw Error('publication baseline failed: ' + JSON.stringify(publicationBaseline));
  const runnerBaseline = run(undefined, 'test/provider-runner.test.js');
  if (runnerBaseline.status !== 0)
    throw Error('runner baseline failed: ' + JSON.stringify(runnerBaseline));
  const rows = [];
  for (const c of cases.filter(
    (c) => !process.argv.slice(2).length || process.argv.slice(2).includes(c.name),
  )) {
    const path = join(scratch, 'ci/vast-broker', c.file),
      original = readFileSync(path, 'utf8');
    try {
      const count = original.split(c.before).length - 1;
      if (!count || (!c.all && count !== 1))
        throw Error('ambiguous edit ' + c.name + ' count=' + count);
      writeFileSync(
        path,
        c.all ? original.replaceAll(c.before, c.after) : original.replace(c.before, c.after),
      );
      let r;
      try {
        r = run(c.pattern, c.test);
      } catch (e) {
        throw new Error(c.name + ': ' + e.message);
      }
      if (r.status !== 1 || r.failures.length < 2)
        throw Error('fewer than two killers: ' + c.name + ' ' + JSON.stringify(r));
      rows.push({ mutant: c.name, kills: r.failures.length, killers: r.failures });
    } finally {
      writeFileSync(path, original);
    }
  }
  process.stdout.write(
    JSON.stringify(
      {
        status: 'PASS',
        baseline_tests: baseline.tests,
        review_tests: reviewBaseline.tests,
        publication_tests: publicationBaseline.tests,
        cases: rows,
      },
      null,
      2,
    ) + '\n',
  );
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
