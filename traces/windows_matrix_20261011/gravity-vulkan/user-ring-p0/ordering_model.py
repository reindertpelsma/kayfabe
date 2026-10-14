#!/usr/bin/env python3
"""Small interleaving counterexample, not a GPU/driver/parser implementation.

It tests the proposal's ordering argument: a reply is not a fence over already
forwarded aliases. The corrected invariant is snapshot/inspection from birth.
"""
import itertools
import json
import unittest

EVENTS = ('publish', 'rewrite', 'set_parse', 'register', 'reply', 'fetch')

def schedules():
    for order in itertools.permutations(EVENTS):
        at = {event: order.index(event) for event in EVENTS}
        # Registration sets PARSE before installing the metadata and replying.
        # The old submission is already published; the hostile guest can change
        # its referenced PB before the GPU actually fetches it.
        if (at['publish'] < at['set_parse'] < at['register'] < at['reply']
                and at['publish'] < at['rewrite'] < at['fetch']
                and at['register'] < at['fetch']):
            yield order

def simulate(order, inspect_from_birth):
    guest_word = 'ordinary'
    registered = False
    staged = None
    inspected = False
    raw_trigger = False
    parse = inspect_from_birth
    for event in order:
        if event == 'publish':
            inspected = parse
            staged = guest_word if parse else None
        elif event == 'rewrite':
            guest_word = 'trigger'
        elif event == 'set_parse':
            parse = True
        elif event == 'register':
            registered = True
        elif event == 'fetch':
            word = staged if inspected else guest_word
            raw_trigger = word == 'trigger' and registered and not inspected
    return raw_trigger

class Ordering(unittest.TestCase):
    def test_reply_before_flag_is_not_the_problem(self):
        for order in schedules():
            self.assertLess(order.index('set_parse'), order.index('reply'))

    def test_holding_reply_does_not_protect_old_mutable_submissions(self):
        cases = list(schedules())
        self.assertTrue(cases)
        self.assertTrue(any(simulate(s, False) for s in cases))

    def test_trigger_can_become_executable_before_registration_reply(self):
        examples = [s for s in schedules() if s.index('fetch') < s.index('reply')]
        self.assertTrue(examples)
        self.assertTrue(any(simulate(s, False) for s in examples))

    def test_immutable_inspection_from_birth_closes_this_alias_race(self):
        for order in schedules():
            self.assertFalse(simulate(order, True), order)

if __name__ == '__main__':
    cases = list(schedules())
    bad = [s for s in cases if simulate(s, False)]
    print(json.dumps(dict(valid_schedules=len(cases), late_parse_counterexamples=len(bad),
          first_counterexample=bad[0],
          birth_snapshot_counterexamples=sum(simulate(s, True) for s in cases),
          scope='ordering model only; not native migration/parser proof')), flush=True)
    unittest.main()
