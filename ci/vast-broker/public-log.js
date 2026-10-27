// Trusted literal renderer; display-only, with the same public fields as the BASE runner.
export const publicLogRenderer = String.raw`import html,json,math,os,re,sys
mode,data_path,log_path=sys.argv[1:]
with open(data_path,encoding='utf-8') as source:
    data=json.load(source)
def number(key,default=0):
    value=data.get(key,default)
    if isinstance(value,(int,float)) and not isinstance(value,bool):
        if math.isfinite(value) and value>=0:
            return value
    return default
def clean(text):
    text=re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]','',str(text))
    text=re.sub(r'[\x00-\x08\x0b-\x1f\x7f-\x9f]','',text)
    return re.sub(r':{2,}',lambda match:' '.join(match[0]),text).replace('##[','# #[')
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
if mode=='live':
    if data.get('cursor',0)==0 and instance!='-':
        print('RENTAL machine='+identifier('machine_id')+' host='+identifier('host_id')+
              ' instance='+instance+' gpu='+gpu.replace(' ','_')+' price='+price)
    print(clean(data.get('log_increment_sanitised','')),end='')
elif mode=='final':
    print('::group::Full bounded log: UNTRUSTED output of the rented box')
    with open(log_path,encoding='utf-8') as source:
        print(clean(source.read(4194304)),end='')
    print('\nDropped bytes: '+str(int(number('bytes_dropped'))))
    print('::endgroup::')
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
            safe=html.escape(value).replace('|','&#124;').replace(chr(96),'&#96;')
            summary.write('| '+key+' | '+safe+' |\n')
        summary.write('\nLog tail (UNTRUSTED output of the rented box):\n\n'+fence+
                      'text\n'+tail+'\n'+fence+'\n')
else:
    raise SystemExit('unsupported render mode')
`;
