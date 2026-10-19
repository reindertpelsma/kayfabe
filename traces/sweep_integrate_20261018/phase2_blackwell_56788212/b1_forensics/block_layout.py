"""Select a verified NTFS partition from lsblk JSON; never infer filesystem type."""
import json,sys

def select_ntfs(layout, node='/dev/nbd15'):
    parts=[p for p in layout['blockdevices'][0].get('children',[]) if p.get('fstype')=='ntfs']
    assert parts,'No NTFS partition discovered; do not guess'
    p=max(parts,key=lambda x:x['size'])
    assert p['size']>1024**3
    return p['path']

if __name__=='__main__':
    print(select_ntfs(json.load(open(sys.argv[1]))))
