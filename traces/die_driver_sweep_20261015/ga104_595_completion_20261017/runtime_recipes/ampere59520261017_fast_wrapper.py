#!/usr/bin/env python3
import os,subprocess,sys
binary=os.environ['AMPERE595_QEMU_BIN']
subprocess.Popen([sys.executable,'/root/ampere595_fast_receipt.py',str(os.getpid()),binary,os.environ['AMPERE595_SOURCE'],os.environ['AMPERE595_LABEL']],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,close_fds=True)
os.execve(binary,[binary]+sys.argv[1:],os.environ)
