// Trusted literal renderer; display-only, with the same public fields as the BASE runner.
export const publicLogRenderer = String.raw`import base64,html,ipaddress,json,math,os,re,sys
mode,data_path,log_path=sys.argv[1:]
with open(data_path,encoding='utf-8') as source:
    data=json.load(source)
def number(key,default=0):
    value=data.get(key,default)
    if isinstance(value,(int,float)) and not isinstance(value,bool):
        if math.isfinite(value) and value>=0:
            return value
    return default
def endpoint_integer(value, base=10):
    value = value.lstrip("0") or "0"
    if len(value) > 40:
        return None
    try:
        return int(value, base)
    except ValueError:
        return None


def endpoint_ipv4(value):
    parts = value.split(".")
    if len(parts) > 4:
        return []
    values = [0]
    for i, part in enumerate(parts):
        if re.fullmatch(r"0x[0-9a-f]+", part, re.I):
            candidates = [endpoint_integer(part[2:], 16)]
        elif re.fullmatch(r"[0-9]+", part):
            candidates = [endpoint_integer(part)]
            if re.fullmatch(r"0[0-7]+", part):
                candidates.append(endpoint_integer(part, 8))
        else:
            candidates = []
        width = 8 * (5 - len(parts)) if i == len(parts) - 1 else 8
        values = [(n << width) + v for n in values for v in candidates if v is not None and v < 1 << width]
    return values


def endpoint_addresses(value):
    if not isinstance(value, str):
        return []
    value = value.removeprefix("[").removesuffix("]").split("%")[0].lower()
    if ":" not in value:
        return endpoint_ipv4(value)
    if "." in value:
        prefix, tail = value.rsplit(":", 1)
        return [address for n in endpoint_ipv4(tail) for address in endpoint_addresses(prefix + ":" + format(n >> 16, "x") + ":" + format(n & 65535, "x"))]
    halves = value.split("::")
    if len(halves) > 2:
        return []
    left = halves[0].split(":") if halves[0] else []
    right = halves[1].split(":") if len(halves) > 1 and halves[1] else []
    count = len(left) + len(right)
    if (count != 8 if len(halves) == 1 else count >= 8):
        return []
    words = left + ["0"] * (8 - count) + right
    if not all(re.fullmatch(r"[0-9a-f]{1,4}", word) for word in words):
        return []
    return [int("".join(word.zfill(4) for word in words), 16)]


def valid_endpoint_values(context):
    return (
        type(context) is dict and set(context) == {"ips", "ports", "overflow"}
        and type(context["overflow"]) is bool
        and type(context["ips"]) is list and len(context["ips"]) <= 128
        and all(isinstance(ip, str) and len(ip) <= 64 and endpoint_addresses(ip) for ip in context["ips"])
        and type(context["ports"]) is list and len(context["ports"]) <= 128
        and all(type(port) is int and 0 < port <= 65535 for port in context["ports"])
    )


def decode_endpoint_values(value):
    if not isinstance(value, str) or len(value) > 16384:
        return None
    try:
        context = json.loads(base64.b64decode(value, validate=True))
    except (ValueError, UnicodeError):
        return None
    return context if valid_endpoint_values(context) else None


def merge_endpoint_values(prior, current):
    ips = list(dict.fromkeys(prior.get("ips", []) + current.get("ips", [])))
    ports = list(dict.fromkeys(prior.get("ports", []) + current.get("ports", [])))
    return {"ips": ips[:128], "ports": ports[:128], "overflow": prior.get("overflow", False) or current.get("overflow", False) or len(ips) > 128 or len(ports) > 128}


def redact_endpoint_values(text, context=None):
    context = context or {}
    numbers = set()
    for ip in context.get("ips", []):
        for n in endpoint_addresses(ip):
            numbers.add(n)
            if n <= 0xffffffff:
                numbers.add(0xffff00000000 + n)
            if n >> 32 == 65535:
                numbers.add(n & 0xffffffff)
    ports = {str(port) for port in context.get("ports", [])}
    def candidate(match):
        return "[redacted]" if context.get("overflow") or any(n in numbers for n in endpoint_addresses(match[0])) else match[0]
    text = re.sub(r"(?<![0-9a-f:.])[0-9a-f:.]*:[0-9a-f:.]+", candidate, text, flags=re.I)
    text = re.sub(r"(?<![0-9])(?:0x[0-9a-f]+|[0-9]+)(?:\.(?:0x[0-9a-f]+|[0-9]+)){1,3}", candidate, text, flags=re.I)
    for ip in context.get("ips", []):
        text = re.sub(re.escape(ip), "[redacted]", text, flags=re.I)
    text = re.sub(r"0x[0-9a-f]+|[0-9a-f]{8,}", lambda m: "[redacted]" if context.get("overflow") or endpoint_integer(re.sub(r"^0x", "", m[0], flags=re.I), 16) in numbers else m[0], text, flags=re.I)
    return re.sub(r"[0-9]+", lambda m: "[redacted]" if context.get("overflow") or (m[0].lstrip("0") or "0") in ports or endpoint_integer(m[0]) in numbers else m[0], text)


def public_ip_prefix(value):
    if not isinstance(value, str) or len(value) > 64 or "%" in value:
        return "unknown"
    try:
        address = ipaddress.ip_address(value)
        if isinstance(address, ipaddress.IPv6Address) and address.ipv4_mapped:
            address = address.ipv4_mapped
        if isinstance(address, ipaddress.IPv4Address):
            return str(ipaddress.ip_network(str(address)+"/24", strict=False).network_address)
        return str(ipaddress.ip_network(str(address)+"/48", strict=False))
    except ValueError:
        return "unknown"


def public_endpoint_match(match):
    prefix = public_ip_prefix(match[1])
    return match[0] if prefix == "unknown" else prefix


def port_operand_end(text, start):
    end = start
    if end < len(text) and text[end] in '+-':
        end += 1
    while end < len(text) and text[end] in ' \t\r\n\v\f':
        end += 1
    digits = end
    while end < len(text) and text[end] in '0123456789':
        end += 1
    if end == digits or (end < len(text) and re.match(r'[A-Za-z0-9_:]', text[end])):
        return None
    return end


def port_separators(text, start):
    while start < len(text) and text[start] in ' \t\r\n\v\f"\'=:':
        start += 1
    return start


def redact_port_fields(text):
    pieces = []
    through = 0
    selectors = r'(?<![A-Za-z0-9])(?:--(?:ssh-)?port\b|-[A-Za-z0-9]*?[opP]|\b(?:ssh[_ -]?(?:port|host)|port)\b)'
    for match in re.finditer(selectors, text, re.I):
        if match.start() < through:
            continue
        start = match.end()
        option = match[0].lower()
        if option.startswith("-") and not option.startswith("--"):
            option = "-" + option[-1]
        if option == '-o':
            start = port_separators(text, start)
            if text[start:start + 4].lower() != 'port':
                continue
            start += 4
        start = port_separators(text, start)
        end = port_operand_end(text, start)
        if end is None:
            continue
        label = '-oPort=' if option == '-o' else '-p ' if option == '-p' else 'ssh_port='
        pieces.append(text[through:match.start()] + label + '[redacted]')
        through = end
    text = ''.join(pieces) + text[through:]
    pieces = []
    through = 0
    endpoints = r'(?<![A-Za-z0-9_.:%-])(\[[^\[\]\s]+\]|[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*):'
    for match in re.finditer(endpoints, text):
        if match.start() < through:
            continue
        host = match[1].removeprefix('[').removesuffix(']')
        prefix = public_ip_prefix(host)
        context_end = match.start()
        while context_end > 0 and text[context_end - 1] in ' \t\r\n\v\f':
            context_end -= 1
        context = text[max(0, context_end - 96):context_end]
        authority = re.search(r'(?:^|\W)(?:ssh|scp|endpoint|host)[=:]?$|://$|@$', context, re.I)
        qualified = '.' in host and re.search(r'[A-Za-z]', host)
        if prefix == 'unknown' and not qualified and not authority:
            continue
        start = port_separators(text, match.end())
        end = port_operand_end(text, start)
        if end is None:
            continue
        pieces.append(text[through:match.start()] + (match[1] + ':[redacted]' if prefix == 'unknown' else prefix))
        through = end
    return ''.join(pieces) + text[through:]


def redact_endpoints(text):
    text = redact_port_fields(text)
    text = re.sub(r'\[?\b((?:[0-9]{1,3}\.){3}[0-9]{1,3})\]?(?::[0-9]{1,5})?(?![0-9])', public_endpoint_match, text)
    return re.sub(r'(?<![A-Za-z0-9_])\[?([0-9a-f]{0,4}(?::[0-9a-f]{0,4}){2,7})\]?(?::[0-9]{1,5})?(?:/48)?(?![A-Za-z0-9_])', public_endpoint_match, text, flags=re.I)


def rental_datacenter(data):
    value=data.get("datacenter")
    return "true" if value is True else "false" if value is False else "unknown"


def rental_prefix(data):
    value=data.get("ip_prefix", "unknown")
    if not isinstance(value,str):
        return "unknown"
    return public_ip_prefix(value.removesuffix("/48"))


def clean(text):
    context=decode_endpoint_values(globals().get("data",{}).get("endpoint_redactions")) or {}
    text=redact_endpoint_values(str(text),context)
    text=re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]','',str(text))
    text=re.sub(r'[\x00-\x08\x0b-\x1f\x7f-\x9f]','',text)
    return redact_endpoint_values(re.sub(r':{2,}',lambda match:' '.join(match[0]),redact_endpoints(redact_endpoint_values(text,context))).replace('##[','# #['),context)
def label(key,default='unknown',limit=128):
    value=data.get(key,default)
    return ' '.join(clean(value).splitlines())[:limit] if isinstance(value,str) else default
def identifier(key):
    value=data.get(key)
    if type(value) is int and 0<value<=9007199254740991:
        return str(value)
    if isinstance(value,str) and re.fullmatch(r'[1-9][0-9]{0,19}',value):
        return value
    return '-'
def gpu_model():
    value=data.get('gpu_model','unknown')
    prefix=r'(?:NVIDIA[ _])?(?:(?:GeForce|Quadro)[ _])?'
    model=r'(?:RTX|GTX)[ _](?:PRO[ _])?[A-Z]{0,2}\d{3,4}[A-Z]{0,2}'
    suffix=r'(?:[ _-][A-Z0-9]{1,16}){0,4}'
    quadro=r'(?:NVIDIA[ _])?Quadro[ _][A-Z]?\d{3,4}'
    die=r'(?:TU|GA|AD|GB)\d{3}|CC_\d{2,3}'
    pattern=r'(?:'+prefix+model+suffix+'|'+quadro+suffix+'|'+die+')'
    if isinstance(value,str) and len(value)<=100 and re.fullmatch(pattern,value,re.I|re.A):
        return value
    return 'unknown'
gpu=gpu_model()
instance=identifier('instance_id' if 'instance_id' in data else 'id')
price=number('dph_total',None)
price='unknown' if price is None else format(price,'.6f')
reference=data.get('host_reference','MISSING')
if reference not in {'GREEN','NOT_GREEN','MISSING'}:
    reference='MISSING'
state=data.get('state','UNAVAILABLE')
if state not in {
    'RUNNING','STOPPING','STOPPED','COMPLETE','NOT_NEEDED','NOT_ELIGIBLE','NEEDS_APPROVAL',
    'RENTING_DISABLED','BUSY','UNAVAILABLE','UNAVAILABLE_BUDGET','UNAUTHORIZED','NOT_FOUND',
    'CREATING','BOOTING','TESTING','GRACE','NONE',
}:
    state='UNAVAILABLE'
verdict=data.get('verdict','UNAVAILABLE')
if verdict not in {'PASS','FAIL','UNAVAILABLE'}:
    verdict='UNAVAILABLE'
def public_print(text='',**kwargs):
    print(redact_endpoint_values(str(text),decode_endpoint_values(data.get('endpoint_redactions')) or {}),**kwargs)
if mode=='live':
    if data.get('cursor',0)==0 and instance!='-':
        country=data.get('country','UNKNOWN')
        country=country if isinstance(country,str) and re.fullmatch(r'UNKNOWN|[A-Z]{2}',country) else 'UNKNOWN'
        public_print('RENTAL instance='+instance+' host='+identifier('host_id')+' machine='+identifier('machine_id')+
              ' datacenter='+rental_datacenter(data)+' country='+country+' ip='+rental_prefix(data)+
              ' gpu='+gpu.replace(' ','_')+' price='+price)
    public_print(clean(data.get('log_increment_sanitised','')),end='')
elif mode=='final':
    public_print('::group::Full bounded log: UNTRUSTED output of the rented box')
    with open(log_path,encoding='utf-8') as source:
        public_print(clean(source.read(4194304)),end='')
    public_print('\nDropped bytes: '+str(int(number('bytes_dropped'))))
    public_print('::endgroup::')
elif mode=='summary':
    tail=clean(data.get('log_tail_sanitised',''))[:8192]
    longest=max([len(x) for x in re.findall(chr(96)+r'+',tail)]+[0])
    fence=chr(96)*max(3,1+longest)
    sha=data.get('sha','unknown')
    if not isinstance(sha,str) or not re.fullmatch('[a-f0-9]{40}',sha):
        sha='unknown'
    end=number('result_at',number('ended_at',number('started_at')))
    duration=max(0,end-number('started_at'))/1000
    explanation='Box reported a structured result'
    if data.get('result_at') is None:
        explanation='No box result: '+state
    rows=[('State',state),('Verdict',verdict),('Summary',explanation),('Tested commit',sha),
          ('GPU model',gpu),('GPU die',label('die',limit=24)),('Vast instance id',instance),
          ('Vast machine id',identifier('machine_id')),('Vast host id',identifier('host_id')),
          ('Offer geolocation (untrusted)',label('geolocation')),('Offer USD/hour',price),
          ('Host driver label (untrusted)',label('host_driver_version',limit=32)),
          ('Host reference (untrusted)',reference),
          ('Create to result seconds',format(duration,'.3f')),
          ('Estimated cost USD',format(number('cost_estimate_usd'),'.6f')),
          ('Dropped log bytes',str(int(number('bytes_dropped'))))]
    with open(os.environ['GITHUB_STEP_SUMMARY'],'a',encoding='utf-8') as summary:
        summary.write('### UNTRUSTED output of the rented box\n\n| Field | Value |\n| --- | --- |\n')
        for key,value in rows:
            safe=html.escape(clean(value)).replace('|','&#124;').replace(chr(96),'&#96;')
            summary.write('| '+key+' | '+safe+' |\n')
        summary.write('\nLog tail (UNTRUSTED output of the rented box):\n\n'+fence+
                      'text\n'+tail+'\n'+fence+'\n')
else:
    raise SystemExit('unsupported render mode')
`;
