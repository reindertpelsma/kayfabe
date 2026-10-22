"""Validate all three process-stop controls without PowerShell array coercion."""
def controls_ok(controls, records):
    if isinstance(controls, dict):
        if set(controls) != {'value', 'Count'} or not isinstance(controls['value'], list):
            return False
        if controls['Count'] != len(controls['value']):
            return False
        controls = controls['value']
    if not isinstance(controls, list) or len(controls) != 3:
        return False
    if [c.get('expected') for c in controls] != [0, 7, 9]:
        return False
    if any(c.get('actual') != c['expected'] for c in controls):
        return False
    if any(type(c.get('pid')) is not int or c['pid'] <= 0 for c in controls):
        return False
    if len({c['pid'] for c in controls}) != 3:
        return False
    return all(any(r.get('pid') == c['pid'] and r.get('exit') == c['expected']
                   and r.get('name') == 'cmd.exe' for r in records) for c in controls)
