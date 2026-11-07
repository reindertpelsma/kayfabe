const prefix = 'KF_DOWNLOAD_METRIC seconds=';
function partialMetric(value) {
  if (prefix.startsWith(value)) return true;
  if (!value.startsWith(prefix)) return false;
  const rest = value.slice(prefix.length);
  const seconds = /^[0-9]{0,6}(?:\.[0-9]{0,6})?/.exec(rest)[0];
  const suffix = rest.slice(seconds.length);
  return (
    ' bytes='.startsWith(suffix) ||
    (suffix.startsWith(' bytes=') && /^[0-9]{0,13}$/.test(suffix.slice(7)))
  );
}
// Bounded calibration claims from instrumented benchmark transfers. Never admission evidence.
export function captureDownloadMetrics(record, text) {
  let tail = record.downloadMetricTail ?? '',
    discard = record.downloadMetricDiscard ?? false;
  for (const part of text.split(/(?<=\n)/)) {
    if (discard) {
      if (part.endsWith('\n')) discard = false;
      continue;
    }
    tail += part;
    if (tail.length > 256) {
      tail = '';
      discard = !part.endsWith('\n');
      continue;
    }
    if (!tail.endsWith('\n')) {
      if (!partialMetric(tail)) {
        tail = '';
        discard = true;
      }
      continue;
    }
    const match =
      /^KF_DOWNLOAD_METRIC seconds=([0-9]{1,6}(?:\.[0-9]{1,6})?) bytes=([0-9]{1,13})\r?\n$/.exec(
        tail,
      );
    tail = '';
    if (!match) continue;
    const seconds = Number(match[1]),
      bytes = Number(match[2]);
    if (seconds > 86400 || bytes > 1e12) continue;
    const measurements = (record.downloadMeasurements ??= {
      source: 'payload_log_claim',
      coverage: 'instrumented_curl_transfers_only',
      advertised_inet_down: record.offerFacts?.inet_down ?? null,
      samples: [],
      sum_transfer_seconds: 0,
      sum_bytes: 0,
      truncated: false,
    });
    if (measurements.samples.length >= 32) {
      measurements.truncated = true;
      continue;
    }
    measurements.samples.push({ seconds, bytes });
    measurements.sum_transfer_seconds += seconds;
    measurements.sum_bytes += bytes;
  }
  record.downloadMetricTail = tail;
  record.downloadMetricDiscard = discard;
}
