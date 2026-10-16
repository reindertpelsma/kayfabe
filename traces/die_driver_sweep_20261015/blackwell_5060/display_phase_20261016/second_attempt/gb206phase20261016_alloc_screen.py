"""Screen public580.65.06 display allocations; never export pointer values."""
import json

def allocation_summary(payload, selector, layouts):
    # The exact public rpc_gsp_rm_alloc_v03_00 header is32bytes; kf-abi's
    # decode_rpc_alloc derives/validates this same shape. No other ABI admitted.
    if layouts['abi']!='580.65.06' or len(payload)<32:return None
    word=lambda off:int.from_bytes(payload[off:off+4],'little')
    if word(12)!=selector or word(24)!=0:return None # opaque/serialized flags
    spec=next((layouts[k] for k in ['pio','dma'] if selector in layouts[k]['classes']),None)
    if spec is None or word(20)!=spec['size'] or len(payload)!=32+spec['size']:return None
    params=payload[32:];fields={}
    for name,(off,width) in spec['fields'].items():
        if width not in [1,2,4,8] or off<0 or off+width>len(params):return None
        value=int.from_bytes(params[off:off+width],'little')
        if name=='pControl':fields['pControl_nonzero']=bool(value)
        else:fields[name]=value
    return {'class':hex(selector),'client':hex(word(0)),'parent':hex(word(4)),
            'handle':hex(word(8)),'body_status':hex(word(16)),
            'params_size':spec['size'],'public_params':fields,
            'screening':'pControl value omitted; only compiled public fields, no raw allocation body'}
