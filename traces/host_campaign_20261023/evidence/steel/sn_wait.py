import subprocess,sys,time
# wait for the GPU to be busy (>=50% for 5 s) then idle (<15% for 8 s); timeout 400 s total. prints start/end
def u():
    return int(subprocess.check_output(["nvidia-smi","--query-gpu=utilization.gpu","--format=csv,noheader,nounits"],text=True).split()[0])
t0=time.time();busy=0;seen=False;idle=0;start=None
while time.time()-t0<400:
    x=u()
    if not seen:
        busy=busy+1 if x>=50 else 0
        if busy>=5: seen=True;start=time.strftime("%H:%M:%S");
    else:
        idle=idle+1 if x<15 else 0
        if idle>=8: print("BUSY_START",start,"END",time.strftime("%H:%M:%S"));sys.exit(0)
    time.sleep(1)
print("TIMEOUT",seen);sys.exit(1)
