import sys
from pathlib import Path
import unittest
sys.path.insert(0,str(Path(__file__).parent))
from shutdown_census import await_shutdown,CensusRefused

class Clock:
    def __init__(self):self.now=0;self.sleeps=[]
    def time(self):return self.now
    def sleep(self,n):self.sleeps.append(n);self.now+=n

class ShutdownCensus(unittest.TestCase):
    def run_case(self,alive,census,xids=lambda:('baseline',)):
        return await_shutdown(lambda:alive(self.c.now),census,xids,('baseline',),self.c.sleep,self.c.time)
    def setUp(self):self.c=Clock()
    def test_owned_exit_after_five_seconds_is_not_a_failure(self):
        self.run_case(lambda t:t<5,lambda:[]);self.assertEqual(self.c.now,5)
    def test_owned_exit_at_sixty_seconds_is_bounded_and_accepted(self):
        self.run_case(lambda t:t<60,lambda:[]);self.assertEqual(self.c.now,60)
    def test_process_list_is_read_again_after_exit_in_flight(self):
        values=iter(([99],[]));self.run_case(lambda t:False,lambda:next(values));self.assertEqual(self.c.sleeps,[5])
    def test_last_of_three_rereads_must_use_a_fresh_clean_state(self):
        values=iter(([99],[99],[99],[]));r=self.run_case(lambda t:False,lambda:next(values));self.assertEqual(self.c.sleeps,[5,5,5]);self.assertEqual(r['rereads'],3)
    def test_owned_qemu_still_alive_at_sixty_seconds_is_fatal(self):
        with self.assertRaises(CensusRefused):self.run_case(lambda t:True,lambda:[])
        self.assertEqual(self.c.now,60)
    def test_other_qemu_after_three_rereads_is_fatal(self):
        with self.assertRaises(CensusRefused):self.run_case(lambda t:False,lambda:[99])
        self.assertEqual(self.c.sleeps,[5,5,5])
    def test_a_later_clean_fifth_read_does_not_extend_the_bound(self):
        values=iter(([99],[99],[99],[99],[]))
        with self.assertRaises(CensusRefused):self.run_case(lambda t:False,lambda:next(values))
        self.assertEqual(self.c.sleeps,[5,5,5])
    def test_new_native32_while_waiting_is_fatal(self):
        with self.assertRaisesRegex(CensusRefused,'Xid'):self.run_case(lambda t:t<10,lambda:[],lambda:('baseline',) if self.c.now<5 else ('baseline','native32'))
    def test_new_xid_after_the_final_clean_census_is_fatal(self):
        seen=[False]
        def census():seen[0]=True;return []
        with self.assertRaisesRegex(CensusRefused,'Xid'):self.run_case(lambda t:False,census,lambda:('baseline','other31') if seen[0] else ('baseline',))
    def test_a_new_fault_before_the_first_read_is_fatal(self):
        with self.assertRaisesRegex(CensusRefused,'Xid'):self.run_case(lambda t:False,lambda:[],lambda:('baseline','32'))
    def test_quiet_shutdown_needs_no_wait(self):
        r=self.run_case(lambda t:False,lambda:[]);self.assertEqual(self.c.now,0);self.assertEqual(r['rereads'],0)

if __name__=='__main__':unittest.main()
