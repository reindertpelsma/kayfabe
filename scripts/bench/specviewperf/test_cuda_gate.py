import unittest
import kayfabe_arm as arm


class CudaGate(unittest.TestCase):
    def gate(self, values):
        calls=[]; values=iter(values)
        result=arm.ensure_cuda(lambda: next(values), lambda: calls.append('reload'))
        return result,calls

    def test_healthy_does_not_reload(self):
        self.assertEqual(self.gate([0])[1], [])

    def test_healthy_records_status(self):
        self.assertEqual(self.gate([0])[0]['before'], 0)

    def test_999_reloads_once(self):
        self.assertEqual(self.gate([999,0])[1], ['reload'])

    def test_recovered_status_is_recorded(self):
        self.assertEqual(self.gate([999,0])[0]['after'], 0)

    def test_persistent_999_stops_after_one_reload(self):
        calls=[]
        with self.assertRaises(RuntimeError):
            arm.ensure_cuda(lambda: 999, lambda: calls.append('reload'))
        self.assertEqual(calls,['reload'])

    def test_failed_recheck_is_not_accepted(self):
        values=iter([999,100])
        with self.assertRaises(RuntimeError):
            arm.ensure_cuda(lambda: next(values), lambda: None)

    def test_other_error_is_not_reloaded(self):
        calls=[]
        with self.assertRaises(RuntimeError):
            arm.ensure_cuda(lambda: 100, lambda: calls.append('reload'))
        self.assertEqual(calls,[])

    def test_other_error_stops(self):
        with self.assertRaises(RuntimeError):
            self.gate([1])

    def test_reload_unloads_dependents_first(self):
        calls=[];arm.reload_nvidia(lambda argv,**kw:calls.append(argv))
        self.assertEqual(calls[0],['systemctl','stop','gdm'])
        self.assertEqual(calls[1],['rmmod','nvidia_uvm','nvidia_drm','nvidia_modeset','nvidia'])

    def test_reload_loads_then_starts_desktop(self):
        calls=[];arm.reload_nvidia(lambda argv,**kw:calls.append(argv))
        self.assertEqual(calls[2],['modprobe','-a','nvidia','nvidia_uvm','nvidia_modeset','nvidia_drm'])
        self.assertEqual(calls[-1],['systemctl','start','gdm'])

    def test_reload_failure_restores_gdm(self):
        calls=[]
        def run(argv,**kw):
            calls.append(argv)
            if argv[0]=='rmmod':raise RuntimeError('busy')
        with self.assertRaises(RuntimeError):arm.reload_nvidia(run)
        self.assertEqual(calls[-1],['systemctl','start','gdm'])

    def test_modprobe_failure_restores_gdm(self):
        calls=[]
        def run(argv,**kw):
            calls.append(argv)
            if argv[0]=='modprobe':raise RuntimeError('load failed')
        with self.assertRaises(RuntimeError):arm.reload_nvidia(run)
        self.assertEqual(calls[-1],['systemctl','start','gdm'])


if __name__=='__main__':unittest.main()
