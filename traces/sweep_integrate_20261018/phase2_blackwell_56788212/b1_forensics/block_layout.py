"""Select a verified NTFS partition from lsblk JSON; never infer filesystem type."""
import json,sys

def select_ntfs(layout, node='/dev/nbd15'):
    nodes=list(layout['blockdevices'])
    parts=[];seen=0
    while nodes:
        p=nodes.pop();seen+=1
        assert seen<=128,'block-layout work bound exceeded'
        nodes.extend(p.get('children',[]))
        path=p.get('path','')
        owned=path.startswith(node+'p') and path[len(node)+1:].isdigit()
        if p.get('fstype')=='ntfs' and owned:
            parts.append(p)
    assert parts,'No NTFS partition discovered; do not guess'
    p=max(parts,key=lambda x:x['size'])
    assert p['size']>1024**3
    return p['path']

if __name__=='__main__':
    print(select_ntfs(json.load(open(sys.argv[1]))))
