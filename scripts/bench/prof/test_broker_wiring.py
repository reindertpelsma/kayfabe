# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
from pathlib import Path
import subprocess,unittest
ROOT=Path(__file__).resolve().parents[3]
class BrokerWiring(unittest.TestCase):
 def code(self):return (ROOT/'broker/nvkvm_broker.c').read_text()
 def test_boot_and_poll_hooks_are_installed(self):
  s=self.code();self.assertEqual(s.count('nb_prof_boot();'),1);self.assertGreaterEqual(s.count('nb_prof_poll();'),2)
 def test_profiler_is_compiled_out_by_default(self):
  s=(ROOT/'broker/Makefile').read_text();self.assertIn('ifeq ($(PROF),1)',s);self.assertIn('OBJS += nb_prof.o',s)
  h=(ROOT/'broker/nb_prof.h').read_text();self.assertIn('#ifdef NB_PROF',h);self.assertIn('#define nb_prof_boot()',h);self.assertIn('((void)0)',h)
 def test_received_and_released_are_wired_to_accepted_existing_edges(self):
  s=self.code();self.assertIn('s->n_commit++;\n        nb_prof_received();',s);self.assertIn('void nb_sink_release',s);self.assertIn('nb_prof_released();',s)
 def test_received_is_after_request_validation_and_before_backend_commit(self):
  s=self.code().split('case NVKVM_BROKER_CMD_COMMIT:',1)[1].split('case NVKVM_BROKER_CMD_NO_CHANGE:',1)[0];self.assertLess(s.index('nb_tick_reply'),s.index('nb_prof_received'));self.assertLess(s.index('nb_prof_received'),s.index('ss->ops->commit'))
 def test_content_classification_reuses_the_existing_commit_book(self):
  s=self.code();self.assertIn('nb_prof_content(present_ns, s->tick.commits_since_present != 0)',s);self.assertIn('s->cfg->stats_line || nb_prof_on()',s)
 def test_feedback_is_captured_before_decimation_with_clock_status(self):
  s=self.code().split('bool nb_sink_refresh(',1)[1].split('void nb_sink_presented(',1)[0];self.assertLess(s.index('nb_prof_feedback('),s.index('nb_tick_refresh('));self.assertIn('NVKVM_BROKER_TICK_F_CLOCK_ONLY',s)
 def test_all_required_optional_hooks_remain_present(self):
  s=self.code()
  for hook in ['nb_prof_boot','nb_prof_poll','nb_prof_received','nb_prof_released','nb_prof_content','nb_prof_feedback']:self.assertIn(hook+'(',s)
 def test_make_switch_invalidates_objects_and_keeps_all_as_the_default(self):
  s=(ROOT/'broker/Makefile').read_text();self.assertIn('.DEFAULT_GOAL := all',s);self.assertIn('$(OBJS): .prof-mode',s);self.assertIn('PROF_ID = off',s);self.assertIn('PROF_ID = prof-',s)
 def test_gnu_make_opt_in_and_default_commands_are_distinct(self):
  on=subprocess.check_output(['make','-B','-n','PROF=1','all'],cwd=ROOT/'broker',text=True)
  off=subprocess.check_output(['make','-B','-n','PROF=0','all'],cwd=ROOT/'broker',text=True)
  self.assertIn('-DNB_PROF',on);self.assertIn('nb_prof.o',on);self.assertNotIn('-DNB_PROF',off);self.assertNotIn('nb_prof.o',off)
 def test_gnu_make_schedules_the_mode_guard_for_both_configurations(self):
  for mode in ['0','1']:
   output=subprocess.check_output(['make','-B','-n','PROF='+mode,'all'],cwd=ROOT/'broker',text=True)
   self.assertIn('test -f .prof-mode',output)
if __name__=='__main__':unittest.main()
