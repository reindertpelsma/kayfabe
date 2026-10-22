import json
from pathlib import Path
import tempfile
import unittest
import init_observer as O

class EarlyInit(unittest.TestCase):
    def scenario(self, rows, *, change=None, alive=True, attempts=4, validator=None):
        temp=tempfile.TemporaryDirectory();self.addCleanup(temp.cleanup)
        phase=Path(temp.name)/'phase.json'
        state={'arm':'on_precision','pid':42,'starttime':123,'exe':'/own/qemu','pci':'0000:00:08'}
        phase.write_text(json.dumps(state));events=[];iterator=iter(rows);count=0
        def fetch():
            nonlocal count
            count+=1;events.append('fetch')
            text=next(iterator)
            if change and count==2:
                other=dict(state);other.update(change);phase.write_text(json.dumps(other))
            return text
        def ready():events.append('ready')
        def record(text):events.append(('record',json.loads(text.split('BW580_RESULT ')[1].splitlines()[0])))
        result=O.observe(phase,'on_precision',fetch,ready,record,validator or (lambda p:True),lambda:alive,attempts=attempts)
        return result,events
    def row(self,code=0,driver='32.0.15.8088',tdr=0):
        return 'BW580_RESULT '+json.dumps({'code':code,'driver':driver,'tdr4101':tdr})+'\n'
    def test_code22_handshake_precedes_code0_record(self):
        ok,e=self.scenario([self.row(22),self.row()]);self.assertTrue(ok);self.assertEqual(e[:3],['fetch','ready','fetch']);self.assertEqual(e[-1][0],'record')
    def test_delayed_code0_is_captured_without_waiting_for_enable_command_return(self):
        ok,e=self.scenario([self.row(22),self.row(22),self.row()]);self.assertTrue(ok);self.assertEqual(e.index('ready'),1);self.assertEqual(len([x for x in e if isinstance(x,tuple)]),1)
    def test_an_old_pid_cannot_supply_the_observation(self):
        with self.assertRaises(ValueError):self.scenario([self.row(22),self.row()],change={'pid':43})
    def test_a_reused_pid_cannot_supply_the_observation(self):
        with self.assertRaises(ValueError):self.scenario([self.row(22),self.row()],change={'starttime':124})
    def test_other_arm_cannot_supply_the_observation(self):
        with self.assertRaises(ValueError):self.scenario([self.row(22),self.row()],change={'arm':'off_c'})
    def test_other_device_cannot_supply_the_observation(self):
        with self.assertRaises(ValueError):self.scenario([self.row(22),self.row()],change={'pci':'0000:00:09'})
    def test_initial_code0_is_not_the_disabled_handshake(self):
        with self.assertRaises(ValueError):self.scenario([self.row()],attempts=1)
    def test_initial_code43_is_not_the_disabled_handshake(self):
        with self.assertRaises(ValueError):self.scenario([self.row(43)],attempts=1)
    def test_wrong_driver_after_enable_is_not_recorded(self):
        ok,e=self.scenario([self.row(22),self.row(driver='32.0.15.8199')],attempts=2);self.assertFalse(ok);self.assertFalse(any(isinstance(x,tuple) for x in e))
    def test_tdr_after_enable_is_not_recorded(self):
        ok,e=self.scenario([self.row(22),self.row(tdr=1)],attempts=2);self.assertFalse(ok);self.assertFalse(any(isinstance(x,tuple) for x in e))
    def test_code43_after_enable_is_not_recorded(self):
        ok,e=self.scenario([self.row(22),self.row(43)],attempts=2);self.assertFalse(ok);self.assertFalse(any(isinstance(x,tuple) for x in e))
    def test_bool_false_is_not_code0(self):
        ok,e=self.scenario([self.row(22),self.row(False)],attempts=2);self.assertFalse(ok);self.assertFalse(any(isinstance(x,tuple) for x in e))
    def test_multiple_results_are_refused(self):
        with self.assertRaises(ValueError):self.scenario([self.row(22),self.row()+self.row()])
    def test_missing_result_is_refused(self):
        with self.assertRaises(ValueError):self.scenario([self.row(22),'no result'])
    def test_oversized_output_is_refused(self):
        with self.assertRaises(ValueError):self.scenario([self.row(22),'x'*65537])
    def test_exhaustion_is_bounded_and_does_not_invent_code0(self):
        ok,e=self.scenario([self.row(22),self.row(22)],attempts=2);self.assertFalse(ok);self.assertEqual(e.count('fetch'),2)
    def test_zero_attempts_are_refused(self):
        with self.assertRaises(ValueError):self.scenario([],attempts=0)
    def test_unbounded_attempts_are_refused(self):
        with self.assertRaises(ValueError):self.scenario([],attempts=121,alive=False)
    def test_a_dead_qemu_cannot_supply_an_observation(self):
        ok,e=self.scenario([],alive=False);self.assertFalse(ok);self.assertEqual(e,[])

    def test_valid_first_result_with_oversized_padding_is_refused(self):
        with self.assertRaises(ValueError):self.scenario([self.row(22)+'x'*65536],attempts=1)
    def test_valid_later_result_with_oversized_padding_is_refused(self):
        with self.assertRaises(ValueError):self.scenario([self.row(22),self.row()+'x'*65536])
    def test_native_owner_refusal_before_query_stops(self):
        def invalid(_):raise ValueError('wrong native owner')
        with self.assertRaises(ValueError):self.scenario([self.row(22),self.row()],validator=invalid)
    def test_native_owner_refusal_after_query_stops(self):
        calls=[]
        def invalid(_):
            calls.append(1)
            if len(calls)==2:raise ValueError('owner changed during query')
        with self.assertRaises(ValueError):self.scenario([self.row(22),self.row()],validator=invalid)

if __name__=='__main__':unittest.main()
