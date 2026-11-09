// Private provider facts; semantic equality, not command spelling, controls redaction.
const TOKEN = '[redacted]';
const LIMIT = 128;
function integer(value, base = 10) {
  value = value.replace(/^0+/, '') || '0';
  if (value.length > 40) return null;
  try {
    return BigInt((base === 16 ? '0x' : base === 8 ? '0o' : '') + value);
  } catch {
    return null;
  }
}
function ipv4(value) {
  const parts = value.split('.');
  if (parts.length > 4) return [];
  let values = [0n];
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i];
    const candidates = /^0x[0-9a-f]+$/i.test(part)
      ? [integer(part.slice(2), 16)]
      : /^[0-9]+$/.test(part)
        ? [integer(part), ...(/^0[0-7]+$/.test(part) ? [integer(part, 8)] : [])]
        : [];
    const width = i === parts.length - 1 ? 8 * (5 - parts.length) : 8;
    values = values.flatMap((n) =>
      candidates
        .filter((v) => v !== null && v < 1n << BigInt(width))
        .map((v) => (n << BigInt(width)) + v),
    );
  }
  return values;
}
function addresses(value) {
  if (typeof value !== 'string') return [];
  value = value
    .replace(/^\[|\]$/g, '')
    .split('%')[0]
    .toLowerCase();
  if (!value.includes(':')) return ipv4(value);
  if (value.includes('.')) {
    const split = value.lastIndexOf(':');
    return ipv4(value.slice(split + 1)).flatMap((n) =>
      addresses(
        value.slice(0, split + 1) + (n >> 16n).toString(16) + ':' + (n & 65535n).toString(16),
      ),
    );
  }
  const halves = value.split('::');
  if (halves.length > 2) return [];
  const left = halves[0] ? halves[0].split(':') : [],
    right = halves[1] ? halves[1].split(':') : [];
  const count = left.length + right.length;
  if (halves.length === 1 ? count !== 8 : count >= 8) return [];
  const words = [...left, ...Array(8 - count).fill('0'), ...right];
  if (!words.every((x) => /^[0-9a-f]{1,4}$/.test(x))) return [];
  return [BigInt('0x' + words.map((x) => x.padStart(4, '0')).join(''))];
}
export function mergeEndpointValues(prior = {}, instance = {}) {
  const result = {
    ips: [...(prior.ips ?? [])],
    ports: [...(prior.ports ?? [])],
    overflow: prior.overflow === true,
  };
  for (const ip of [instance.public_ipaddr, instance.ssh_host])
    if (
      typeof ip === 'string' &&
      ip.length <= 64 &&
      addresses(ip).length &&
      !result.ips.includes(ip)
    )
      result.ips.push(ip.split('%')[0]);
  const port = instance.ssh_port;
  if (
    (typeof port === 'number' || (typeof port === 'string' && /^[0-9]+$/.test(port))) &&
    Number.isInteger(Number(port)) &&
    Number(port) > 0 &&
    Number(port) <= 65535 &&
    !result.ports.includes(Number(port))
  )
    result.ports.push(Number(port));
  if (result.ips.length > LIMIT || result.ports.length > LIMIT) result.overflow = true;
  result.ips = result.ips.slice(0, LIMIT);
  result.ports = result.ports.slice(0, LIMIT);
  return result;
}
export function redactEndpointValues(text, context = {}) {
  const numbers = new Set();
  for (const ip of context?.ips ?? [])
    for (const n of addresses(ip)) {
      numbers.add(n.toString());
      if (n <= 0xffffffffn) numbers.add((0xffff00000000n + n).toString());
      if (n >> 32n === 65535n) numbers.add((n & 0xffffffffn).toString());
    }
  const ports = new Set((context?.ports ?? []).map(String));
  const matches = (n) => n !== null && numbers.has(n.toString());
  const candidate = (value) =>
    context?.overflow || addresses(value).some(matches) ? TOKEN : value;
  text = text.replace(/(?<![0-9a-f:.])[0-9a-f:.]*:[0-9a-f:.]+/gi, candidate);
  text = text.replace(
    /(?<![0-9])(?:0x[0-9a-f]+|[0-9]+)(?:\.(?:0x[0-9a-f]+|[0-9]+)){1,3}/gi,
    candidate,
  );
  // Exact spelling also handles a known address embedded in non-address text.
  for (const ip of context?.ips ?? [])
    text = text.replace(new RegExp(ip.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'gi'), TOKEN);
  text = text.replace(/0x[0-9a-f]+|[0-9a-f]{8,}/gi, (value) => {
    const n = integer(value.replace(/^0x/i, ''), 16);
    return context?.overflow || matches(n) ? TOKEN : value;
  });
  return text.replace(/[0-9]+/g, (value) =>
    context?.overflow || ports.has(value.replace(/^0+/, '') || '0') || matches(integer(value))
      ? TOKEN
      : value,
  );
}
