#!/usr/bin/env python3
"""A private mock agent checks framing and failure handling; no guest or GPU."""
import base64
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import unittest

HELPER=Path(__file__).resolve().parents[1]/'diesweep/qga_file_put.py'


class FilePut(unittest.TestCase):
    def transfer(self, mode):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);source=root/'probe.exe';data=bytes(range(256))*400
            source.write_bytes(data);sock=root/'qga.sock'
            server=socket.socket(socket.AF_UNIX);server.bind(str(sock));server.listen(1)
            server.settimeout(5);observed={'writes':0,'data':bytearray(),'connections':0}
            def serve():
                try:
                    connection,_=server.accept();observed['connections']+=1
                    if mode=='lost-sync':
                        connection.recv(4096)
                        connection.close()
                        connection,_=server.accept();observed['connections']+=1
                    with connection:
                        connection.settimeout(5);reader=connection.makefile('rb')
                        first=reader.readline(100000);observed['reset']=first.startswith(b'\xff')
                        command=json.loads(first.lstrip(b'\xff'));token=command['arguments']['id']
                        if mode=='stale':
                            connection.sendall(b'{"error":{"desc":"old parse error"}}\nnot-json\n')
                        connection.sendall(json.dumps({'return':token}).encode()+b'\n')
                        while True:
                            raw=reader.readline(100000)
                            if not raw:break
                            message=json.loads(raw);name=message['execute'];args=message['arguments']
                            reply={}
                            if name=='guest-file-open':reply=9;observed['destination']=args['path']
                            if name=='guest-file-write':
                                chunk=base64.b64decode(args['buf-b64'],validate=True)
                                observed['writes']+=1;observed['data'].extend(chunk)
                                if mode=='lost':return
                                reply={'count':len(chunk)-(mode=='short')}
                            if name=='guest-file-close':
                                observed['closes']=observed.get('closes',0)+1
                                if mode=='close-error' and observed['closes']==1:
                                    connection.sendall(b'{"error":{"desc":"uncertain close response"}}\n')
                                    continue
                            connection.sendall(json.dumps({'return':reply}).encode()+b'\n')
                except (BrokenPipeError,ConnectionResetError):
                    pass
                except BaseException as error:
                    observed['error']=repr(error)
            thread=threading.Thread(target=serve);thread.start()
            try:
                result=subprocess.run([sys.executable,str(HELPER),str(sock),str(source),
                    r'C:\ProgramData\KayfabeSweep\probe.exe'],capture_output=True,timeout=8)
                thread.join(timeout=6);self.assertFalse(thread.is_alive())
                self.assertNotIn('error',observed,observed)
                return result,observed,data
            finally:
                server.close()

    def test_fresh_sync_discards_stale_error_and_garbage_before_writing(self):
        result,observed,data=self.transfer('stale')
        self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        self.assertTrue(observed['reset'])
        self.assertEqual(observed['data'],data)
        self.assertEqual(observed['destination'],r'C:\ProgramData\KayfabeSweep\probe.exe')

    def test_short_write_fails_before_the_next_chunk(self):
        result,observed,_=self.transfer('short')
        self.assertNotEqual(result.returncode,0)
        self.assertEqual(observed['writes'],1)

    def test_lost_write_reply_is_not_replayed_on_a_new_connection(self):
        result,observed,_=self.transfer('lost')
        self.assertNotEqual(result.returncode,0)
        self.assertEqual(observed['writes'],1)
        self.assertEqual(observed['connections'],1)

    def test_source_larger_than_four_mib_is_refused_before_connecting(self):
        with tempfile.TemporaryDirectory() as tmp:
            source=Path(tmp)/'large'
            with source.open('wb') as file:file.truncate(4*1024*1024+1)
            result=subprocess.run([sys.executable,str(HELPER),str(Path(tmp)/'absent.sock'),
                str(source),r'C:\probe.exe'],capture_output=True,timeout=3)
            self.assertNotEqual(result.returncode,0)
            self.assertIn(b'REFUSED source size',result.stdout+result.stderr)

    def test_uncertain_close_is_not_replayed_on_the_same_handle(self):
        result,observed,_=self.transfer('close-error')
        self.assertNotEqual(result.returncode,0)
        self.assertEqual(observed['closes'],1)

    def test_lost_readonly_sync_can_reconnect_before_any_file_command(self):
        result,observed,data=self.transfer('lost-sync')
        self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        self.assertEqual(observed['connections'],2)
        self.assertEqual(observed['data'],data)


if __name__=='__main__':
    unittest.main()
