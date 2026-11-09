export function ipPrefix(value) {
  if (typeof value !== 'string' || value.length > 64) return 'unknown';
  if (/^(?:[0-9]{1,3}\.){3}[0-9]{1,3}$/.test(value)) {
    const parts = value.split('.');
    if (parts.some((p) => Number(p) > 255 || String(Number(p)) !== p)) return 'unknown';
    return parts.slice(0, 3).join('.') + '.0';
  }
  if (!/^[0-9a-f:.]+$/i.test(value) || !value.includes(':')) return 'unknown';
  try {
    const host = new URL('http://[' + value + ']/').hostname.slice(1, -1);
    const [left, right] = host.split('::'),
      a = left ? left.split(':') : [],
      b = right ? right.split(':') : [];
    const groups = host.includes('::')
      ? [...a, ...Array(8 - a.length - b.length).fill('0'), ...b]
      : a;
    if (groups.length !== 8) return 'unknown';
    if (
      groups.slice(0, 5).every((g) => parseInt(g, 16) === 0) &&
      parseInt(groups[5], 16) === 65535
    ) {
      const n = parseInt(groups[6], 16),
        m = parseInt(groups[7], 16);
      return [n >> 8, n & 255, m >> 8, 0].join('.');
    }
    return (
      new URL(
        'http://[' +
          groups
            .slice(0, 3)
            .map((g) => parseInt(g, 16).toString(16))
            .join(':') +
          '::]/',
      ).hostname.slice(1, -1) + '/48'
    );
  } catch {
    return 'unknown';
  }
}
function maskedEndpoint(value, ip) {
  const prefix = ipPrefix(ip);
  return prefix === 'unknown' ? value : prefix;
}
function portOperandEnd(text, start) {
  let end = start;
  if (text[end] === '+' || text[end] === '-') end++;
  while (/[ \t\r\n\v\f]/.test(text[end] ?? '')) end++;
  const digits = end;
  while (end < text.length && text[end] >= '0' && text[end] <= '9') end++;
  return end === digits || /[A-Za-z0-9_:]/.test(text[end] ?? '') ? null : end;
}
function portSeparators(text, start) {
  while (start < text.length && /[ \t\r\n\v\f"'=:]/.test(text[start])) start++;
  return start;
}
function redactPortFields(text) {
  let out = '',
    through = 0;
  const selectors =
    /(?<![A-Za-z0-9])(?:--(?:ssh-)?port\b|-o|-[pP]|\b(?:ssh[_ -]?(?:port|host)|port)\b)/gi;
  for (const match of text.matchAll(selectors)) {
    if (match.index < through) continue;
    let start = match.index + match[0].length;
    const option = match[0].toLowerCase();
    if (option === '-o') {
      start = portSeparators(text, start);
      if (text.slice(start, start + 4).toLowerCase() !== 'port') continue;
      start += 4;
    }
    start = portSeparators(text, start);
    const end = portOperandEnd(text, start);
    if (end === null) continue;
    const label = option === '-o' ? '-oPort=' : option === '-p' ? '-p ' : 'ssh_port=';
    out += text.slice(through, match.index) + label + '[redacted]';
    through = end;
  }
  text = out + text.slice(through);
  out = '';
  through = 0;
  const endpoints = /(?<![A-Za-z0-9_.:%-])(\[[^\[\]\s]+\]|[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*):/g;
  for (const match of text.matchAll(endpoints)) {
    if (match.index < through) continue;
    const host = match[1].replace(/^\[|\]$/g, '');
    const prefix = ipPrefix(host);
    let contextEnd = match.index;
    while (contextEnd > 0 && /[ \t\r\n\v\f]/.test(text[contextEnd - 1])) contextEnd--;
    const context = text.slice(Math.max(0, contextEnd - 96), contextEnd);
    const authority = /(?:^|\W)(?:ssh|scp|endpoint|host)[=:]?$|:\/\/$|@$/i.test(context);
    const qualified = host.includes('.') && /[A-Za-z]/.test(host);
    if (prefix === 'unknown' && !qualified && !authority) continue;
    const start = portSeparators(text, match.index + match[0].length);
    const end = portOperandEnd(text, start);
    if (end === null) continue;
    out +=
      text.slice(through, match.index) + (prefix === 'unknown' ? match[1] + ':[redacted]' : prefix);
    through = end;
  }
  return out + text.slice(through);
}
export function redactEndpoints(text) {
  text = redactPortFields(text);
  return text
    .replace(/\[?\b((?:[0-9]{1,3}\.){3}[0-9]{1,3})\]?(?::[0-9]{1,5})?(?![0-9])/g, (v, ip) =>
      maskedEndpoint(v, ip),
    )
    .replace(
      /(?<![A-Za-z0-9_])\[?([0-9a-f]{0,4}(?::[0-9a-f]{0,4}){2,7})\]?(?::[0-9]{1,5})?(?:\/48)?(?![A-Za-z0-9_])/gi,
      (v, ip) => maskedEndpoint(v, ip),
    );
}
export function rentalLine(r) {
  const id = (v) => (Number.isSafeInteger(Number(v)) && Number(v) > 0 ? String(v) : '-');
  const gpu =
    typeof r.gpu === 'string' ? r.gpu.replace(/[^A-Za-z0-9_]/g, '_').slice(0, 100) : 'unknown';
  const price = Number.isFinite(r.dphTotal) ? r.dphTotal.toFixed(6) : 'unknown';
  return (
    'RENTAL instance=' +
    id(r.id) +
    ' host=' +
    id(r.hostId) +
    ' machine=' +
    id(r.machineId) +
    ' datacenter=' +
    (r.datacenter === true ? 'true' : r.datacenter === false ? 'false' : 'unknown') +
    ' country=' +
    (/^[A-Z]{2}$/.test(r.country) ? r.country : 'UNKNOWN') +
    ' ip=' +
    (r.ipPrefix ?? 'unknown') +
    ' gpu=' +
    gpu +
    ' price=' +
    price
  );
}
