import unittest
from lease_ownership import may_rent

class LeaseOwnership(unittest.TestCase):
    def test_other_ada_lane_does_not_consume_our_one_box(self):self.assertTrue(may_rent([{'id':900,'label':'other-Ada'}],{700}))
    def test_another_owners_box_does_not_block_a_closed_lane(self):self.assertTrue(may_rent([{'id':901,'label':'other'}],{701,702}))
    def test_our_own_active_box_blocks_a_second_lease(self):self.assertFalse(may_rent([{'id':700}],{700}))
    def test_mixed_account_still_blocks_if_any_own_box_is_live(self):self.assertFalse(may_rent([{'id':900},{'id':701}],{700,701}))
    def test_a_different_label_does_not_hide_our_own_id(self):self.assertFalse(may_rent([{'id':702,'label':'other'}],{702}))
    def test_foreign_label_collision_does_not_transfer_ownership(self):self.assertTrue(may_rent([{'id':902,'label':'our-lane'}],{700}))
    def test_empty_account_is_admitted(self):self.assertTrue(may_rent([],{700}))

if __name__=='__main__':unittest.main()
