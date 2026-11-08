#!/usr/bin/env python3
"""GPU-free receipt transport; no executor, VM, provider or network is used."""
import copy
import base64
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('report', Path(__file__).with_name('vast_report.py'))
report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(report)

class MatrixReceipts(unittest.TestCase):
    def proof(self, subrun=False):
        drivers = {'host_driver':'580.94.02','linux_guest_driver':'580.94.02','windows_guest_driver':'580.88'}
        rows = [{'id':'full','drivers':drivers,'windows_guest_abi':'580.65.06','verdict':'PASS'}]
        if subrun:
            rows.append({'id':'windows_subrun_580_97','drivers':{**drivers,'windows_guest_driver':'580.97'},'windows_guest_abi':'580.65.06','verdict':'PASS'})
        return {'schema_version':1,'die':'AD104','rows':rows}
    def doc(self, subrun=False):
        return {'verdict':'PASS','suites':[{'name':'cuda_fat','pass':4,'fail':0,'skip':0}],
                'hardware':{'gpu_name':'RTX_4070','compute_cap':890,'driver_version':'580.94.02'},
                'matrix_receipts':self.proof(subrun)}
    def read(self, doc, wire=False, requested=True):
        with tempfile.TemporaryDirectory(prefix='kf-J2-receipts-') as directory:
            path=Path(directory)/'result.json';path.write_text(json.dumps(doc))
            if not wire:return report.read_result_file(path)
            env={'KF_NONCE':'b'*32,'KF_SHA':'a'*40,'KF_RUNNER_NONCE':'f'*64}
            if requested:env['KF_RELEASE_SUBRUNS']='fixture'
            return report.final_result(env,0,False,None,path)
    def test_full_receipts_are_read(self):
        self.assertEqual(self.read(self.doc())['matrix_receipts'],self.proof())
    def test_subrun_receipts_are_read(self):
        self.assertEqual(self.read(self.doc(True))['matrix_receipts'],self.proof(True))
    def test_full_receipts_are_forwarded(self):
        self.assertEqual(self.read(self.doc(),wire=True)['matrix_receipts'],self.proof())
    def test_subrun_receipts_are_forwarded(self):
        self.assertEqual(self.read(self.doc(True),wire=True)['matrix_receipts'],self.proof(True))
    def test_unrequested_full_receipts_do_not_extend_old_wire(self):
        self.assertNotIn('matrix_receipts',self.read(self.doc(),wire=True,requested=False))
    def test_unrequested_subrun_receipts_do_not_extend_old_wire(self):
        self.assertNotIn('matrix_receipts',self.read(self.doc(True),wire=True,requested=False))
    def test_executable_full_plan_transport_is_understood(self):
        raw=base64.urlsafe_b64encode(json.dumps({'status':'executable','subruns':[],'cross_subrun':None}).encode()).decode().rstrip('=')
        self.assertTrue(report.release_subruns_json(raw))
    def test_executable_cross_plan_transport_is_understood(self):
        raw=base64.urlsafe_b64encode(json.dumps({'status':'executable','subruns':[],'cross_subrun':{}}).encode()).decode().rstrip('=')
        self.assertTrue(report.release_subruns_json(raw))
    def test_duplicate_receipt_is_refused(self):
        doc=self.doc();doc['matrix_receipts']['rows']*=2
        with self.assertRaises(report.ResultError):self.read(doc)
    def test_driver_text_is_refused(self):
        doc=self.doc();doc['matrix_receipts']['rows'][0]['drivers']['host_driver']='https://private.invalid'
        with self.assertRaises(report.ResultError):self.read(doc)
    def test_unknown_field_is_refused(self):
        doc=self.doc();doc['matrix_receipts']['rows'][0]['command']='arbitrary'
        with self.assertRaises(report.ResultError):self.read(doc)
    def test_boolean_schema_is_refused(self):
        doc=self.doc();doc['matrix_receipts']['schema_version']=True
        with self.assertRaises(report.ResultError):self.read(doc)
    def test_failed_receipt_cannot_claim_pass(self):
        doc=self.doc();doc['matrix_receipts']['rows'][0]['verdict']='FAIL'
        with self.assertRaises(report.ResultError):self.read(doc)
    def test_retry_receipt_cannot_claim_pass(self):
        doc=self.doc();doc['matrix_receipts']['rows'][0]['verdict']='RETRY'
        with self.assertRaises(report.ResultError):self.read(doc)

if __name__=='__main__':unittest.main()
