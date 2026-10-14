import faulthandler, os, sys, time
faulthandler.enable()
faulthandler.dump_traceback_later(30,repeat=True)
t=time.monotonic()
def step(s): print('DIAG',round(time.monotonic()-t,3),s,flush=True)
step('python '+sys.version)
lib=r'C:\kfapps\py\Lib\site-packages\torch\lib'
keep=os.add_dll_directory(lib)
os.environ['PATH']=lib+os.pathsep+os.environ.get('PATH','')
step('before import torch')
import torch
step('torch imported '+torch.__version__)
step('before cuda.is_available')
print('CUDA_AVAILABLE',torch.cuda.is_available(),flush=True)
step('before device name')
print('DEVICE',torch.cuda.get_device_name(0),flush=True)
step('before GPU tensor')
x=torch.arange(1024,device='cuda'); y=x*3+14
assert y.cpu().tolist()==[i*3+14 for i in range(1024)]
step('GPU readback passed')
faulthandler.cancel_dump_traceback_later()
