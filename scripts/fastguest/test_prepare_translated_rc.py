#!/usr/bin/env python3
import unittest
from prepare_translated_rc import patch_guest

TAIL = """    return krcErrorSendEventNotifications_HAL(pGpu, pKernelRc,
        pKernelChannel,
        rmEngineType,            // unused on kernel side
        rpc_params->exceptLevel, // unused on kernel side
        rpc_params->exceptType,
        rpc_params->scope,
        rpc_params->partitionAttributionId,
        NV_FALSE                 // unused on kernel side
        );"""
SOURCE = "before\nstatic NV_STATUS\n_kgspRpcRCTriggered\n(\n    OBJGPU *pGpu,\n    OBJRPC *pRpc\n)\n{\n    RPC_PARAMS(rc_triggered, _v17_02);\n"+TAIL+"\n}\n\nafter\n"


class GuestPatchTests(unittest.TestCase):
    def test_observation_is_after_notification(self):
        p=patch_guest(SOURCE)
        self.assertGreater(p.index('"KF_RC_EVENT'),p.index('krcErrorSendEventNotifications_HAL'))
    def test_event_is_logged_only_after_real_success(self):
        p=patch_guest(SOURCE)
        self.assertIn('if (status == NV_OK)',p)
        self.assertIn('return status;',p)
        self.assertGreater(p.index('"KF_RC_EVENT'),p.index('if (status == NV_OK)'))
    def test_event_carries_real_message_identity(self):
        p=patch_guest(SOURCE)
        self.assertIn('rpc_params->chid',p)
        self.assertIn('rpc_params->nv2080EngineType',p)
        self.assertIn('rpc_params->exceptType',p)
        self.assertIn('rpc_params->scope',p)
    def test_other_code_preserved(self):
        p=patch_guest(SOURCE)
        self.assertTrue(p.startswith('before\nstatic NV_STATUS'))
        self.assertTrue(p.endswith('}\n\nafter\n'))
    def test_unrecognized_version_refused(self):
        with self.assertRaises(ValueError):patch_guest(SOURCE.replace('return krcErrorSendEventNotifications_HAL','return another_function'))
    def test_multiple_candidate_bodies_refused(self):
        with self.assertRaises(ValueError):patch_guest(SOURCE+SOURCE)
    def test_already_instrumented_source_refused(self):
        with self.assertRaises(ValueError):patch_guest(patch_guest(SOURCE))


if __name__=='__main__':unittest.main()
