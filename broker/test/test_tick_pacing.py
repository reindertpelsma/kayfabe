#!/usr/bin/env python3
"""The exact-decimation fixture must wait for each accepted wire event, not a delay."""
import unittest
import struct
import test_tick as tick

class Wire:
    def __init__(self,div):self.div=div;self.index=0;self.pending=[]
    def feed(self,line):
        if self.index%self.div==0:
            self.pending=[struct.pack('<HHIIIQ',tick.EV_TICK,0,self.index,self.index,1,self.index)]
        self.index+=1
    def settimeout(self,value):pass
    def recv(self,count):
        if not self.pending:raise AssertionError('read before an accepted feedback')
        packet=self.pending.pop(0)
        self.assert_size=count
        return packet

class PacedFeedback(unittest.TestCase):
    def test_peer_acknowledges_every_accepted_feedback_before_the_next(self):
        wire=Wire(1)
        packets=tick.paced_roundtrip(wire,wire,['t']*3,1)
        self.assertEqual([tick.Tick(packet).present for packet in packets],[0,1,2])
        self.assertEqual(wire.pending,[])

    def test_decimated_feedback_cannot_replace_the_previous_wire_tick(self):
        wire=Wire(2)
        packets=tick.paced_roundtrip(wire,wire,['t']*5,2)
        self.assertEqual([tick.Tick(packet).present for packet in packets],[0,2,4])
        self.assertEqual(wire.pending,[])

if __name__=='__main__':unittest.main()
