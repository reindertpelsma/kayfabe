import unittest
from media_wait import await_media,MediaRefused

class MediaWait(unittest.TestCase):
    def setUp(self):self.now=0
    def sleep(self,n):self.now+=n
    def run_case(self,ready,alive=lambda:True,deadline=1200):return await_media(ready,alive,lambda:self.now,self.sleep,deadline)
    def test_a_live_verified_fetch_after_ten_minutes_is_accepted_within_budget(self):self.assertTrue(self.run_case(lambda:self.now>=605))
    def test_a_completed_fetch_after_ten_minutes_uses_its_verified_success_receipt(self):self.assertTrue(self.run_case(lambda:self.now>=900,lambda:self.now<900))
    def test_a_dead_fetch_without_success_is_fatal(self):
        with self.assertRaisesRegex(MediaRefused,'exited'):self.run_case(lambda:False,lambda:False)
    def test_a_fetch_that_dies_while_waiting_is_fatal(self):
        with self.assertRaisesRegex(MediaRefused,'exited'):self.run_case(lambda:False,lambda:self.now<5)
        self.assertEqual(self.now,5)
    def test_the_remaining_lease_budget_bounds_an_active_fetch(self):
        with self.assertRaises(MediaRefused):self.run_case(lambda:False,deadline=15)
        self.assertEqual(self.now,15)
    def test_a_smaller_budget_is_not_extended_by_a_live_process(self):
        with self.assertRaises(MediaRefused):self.run_case(lambda:False,deadline=5)
        self.assertEqual(self.now,5)
    def test_verified_ready_media_needs_no_sleep(self):self.assertTrue(self.run_case(lambda:True));self.assertEqual(self.now,0)

if __name__=='__main__':unittest.main()
