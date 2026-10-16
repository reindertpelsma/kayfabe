#!/usr/bin/env python3
import os,subprocess,sys
binary=os.environ['ADA2BUILD_QEMU_BIN']
subprocess.Popen([sys.executable,'/root/ada2build_fast_receipt590.py',str(os.getpid()),binary,os.environ['ADA2BUILD_SOURCE'],os.environ['ADA2BUILD_LABEL']],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,close_fds=True)
os.execve(binary,[binary]+sys.argv[1:],os.environ)
