#!/usr/bin/env python3
import unittest
from translated_rc_guard import Refused, check
from test_translated_rc_guard import fixture


def native():
    m,_,_,j=fixture();m['mode']='native';m['detail']=None
    log=('FAULT_MARK=translated_rc_native run=linux-rc-1 pid=123 host=0x19 channel=0x19 source='+m['source']+' word=0xc0000000 unix_ns=2000000000\n'
         'NATIVE_RC_RESULT run=linux-rc-1 code=32 status=0xffff event=1 bystander=1 result=PASS\n')
    return m,log,'',j.replace('name=qemu-system-x86','name=kf-rc-native')


class NativeCaptureTests(unittest.TestCase):
    def test_characterization_freezes_the_native_detail(self):
        r=check(*native());self.assertEqual(r['verdict'],'NATIVE_ONLY');self.assertEqual(r['detail'],'intr 00000010')
    def test_native_proof_never_claims_guest_acceptance(self):
        r=check(*native());self.assertNotEqual(r['verdict'],'PASS');self.assertEqual(r['xid_count'],1)
    def reject(self,old,new,index):
        data=list(native());data[index]=data[index].replace(old,new)
        with self.assertRaises(Refused):check(*data)
    def test_another_process_is_not_a_control(self):self.reject('pid=123','pid=124',3)
    def test_another_channel_is_not_a_control(self):self.reject('channel 0x00000019','channel 0x00000020',3)
    def test_missing_event_rejected(self):self.reject('event=1','event=0',1)
    def test_failed_bystander_rejected(self):self.reject('bystander=1','bystander=0',1)
    def test_wrong_word_rejected(self):self.reject('word=0xc0000000','word=0x0',1)
    def test_failed_native_result_rejected(self):self.reject('result=PASS','result=FAIL',1)
    def test_source_mismatch_rejected(self):self.reject('source='+'a'*40,'source='+'b'*40,1)
    def test_additional_host_fault_rejected(self):
        m,q,g,j=native()
        with self.assertRaises(Refused):check(m,q,g,j+j)
    def test_native_result_cannot_replace_guest_capture(self):
        m,q,g,j=native();m['mode']='on';m['detail']='intr 00000010'
        with self.assertRaises(Refused):check(m,q,g,j)


if __name__=='__main__':unittest.main()
