"""NVIDIA cannot load while vfio-pci owns its only device."""
import unittest
from unittest.mock import patch
import vfio_session


class RestoreOrderTests(unittest.TestCase):
    def exercise(self, bdf, target):
        state={'driver':'vfio-pci','loaded':False}; calls=[]
        def write(path,value):
            calls.append(('write',str(path),value))
            if str(path).endswith('driver/unbind'):state['driver']='unbound'
            if str(path)=='/sys/bus/pci/drivers_probe':
                self.assertTrue(state['loaded'])
                state['driver']=target
        def run(*args,**kwargs):
            calls.append(args)
            if args[0]=='modprobe':
                self.assertEqual(state['driver'],'unbound','detach vfio before loading native driver')
                state['loaded']=True
        with patch.object(vfio_session,'driver',side_effect=lambda _:state['driver']),patch.object(vfio_session,'write',side_effect=write),patch.object(vfio_session,'run',side_effect=run):
            vfio_session.restore_device(bdf,target)
        self.assertEqual(state['driver'],target)
        return calls

    def test_nvidia_unbinds_vfio_before_module_load(self):
        self.exercise(vfio_session.GPU,'nvidia')

    def test_audio_unbinds_vfio_before_module_load(self):
        self.exercise(vfio_session.AUDIO,'snd_hda_intel')


if __name__=='__main__':unittest.main()
