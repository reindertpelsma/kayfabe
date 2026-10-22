// Trusted literal renderer, embedded verbatim in the workflow; never fetched from a PR.
export const publicLogRenderer=String.raw`import html,json,math,os,re,sys
mode,data_path,log_path=sys.argv[1:]
with open(data_path,encoding='utf-8') as source: data=json.load(source)
def number(key,default=0):
    value=data.get(key,default)
    return value if isinstance(value,(int,float)) and not isinstance(value,bool) and math.isfinite(value) and value>=0 else default
def clean(text):
    text=re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]','',str(text))
    text=re.sub(r'[\x00-\x08\x0b-\x1f\x7f-\x9f]','',text)
    return re.sub(r':{2,}',lambda match:' '.join(match[0]),text).replace('##[','# #[')
state=data.get('state','UNAVAILABLE')
if state not in {'RUNNING','STOPPING','STOPPED','COMPLETE','NOT_NEEDED','NOT_ELIGIBLE','NEEDS_APPROVAL','RENTING_DISABLED','BUSY','UNAVAILABLE','UNAVAILABLE_BUDGET','UNAUTHORIZED','NOT_FOUND'}: state='UNAVAILABLE'
verdict=data.get('verdict','UNAVAILABLE')
if verdict not in {'PASS','FAIL','UNAVAILABLE'}: verdict='UNAVAILABLE'
if mode=='live':
    print(clean(data.get('log_increment_sanitised','')),end='')
elif mode=='final':
    print('::group::Full bounded log: UNTRUSTED output of the rented box')
    with open(log_path,encoding='utf-8') as source: print(clean(source.read(4194304)),end='')
    print('\nDropped bytes: '+str(int(number('bytes_dropped'))))
    print('::endgroup::')
elif mode=='summary':
    tail=clean(data.get('log_tail_sanitised',''))[:8192]
    fence=chr(96)*max(3,1+max([len(x) for x in re.findall(chr(96)+r'+',tail)]+[0]))
    gpu=data.get('gpu_model','unknown')
    if gpu not in {'RTX 4070','RTX 4070 Super','RTX 4080','RTX 4090','RTX_4070','RTX_4070_Super','RTX_4080','RTX_4090'}: gpu='unknown'
    sha=data.get('sha','unknown')
    if not isinstance(sha,str) or not re.fullmatch('[a-f0-9]{40}',sha): sha='unknown'
    duration=max(0,(number('result_at',number('ended_at',number('started_at')))-number('started_at'))/1000)
    rows=[('State',state),('Verdict',verdict),('Summary','Box reported a structured result' if data.get('result_at') is not None else 'No box result: '+state),('Tested commit',sha),('GPU model',gpu),('Create to result seconds',format(duration,'.3f')),('Estimated cost USD',format(number('cost_estimate_usd'),'.6f')),('Dropped log bytes',str(int(number('bytes_dropped'))))]
    with open(os.environ['GITHUB_STEP_SUMMARY'],'a',encoding='utf-8') as summary:
        summary.write('### UNTRUSTED output of the rented box\n\n| Field | Value |\n| --- | --- |\n')
        for key,value in rows: summary.write('| '+key+' | '+html.escape(value).replace('|','&#124;')+' |\n')
        summary.write('\nLog tail (UNTRUSTED output of the rented box):\n\n'+fence+'text\n'+tail+'\n'+fence+'\n')
else: raise SystemExit('unsupported render mode')
`;
