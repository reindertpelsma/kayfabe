"""Q1's approved Xid epoch; historical records never authorize fresh faults."""
from pathlib import Path
import re


def delta(journal, history):
    """Keep every fresh record, refusing any changed/lost historical prefix."""
    lines = tuple(line for line in journal.splitlines() if 'NVRM: Xid' in line)
    history = tuple(history)
    if lines[:len(history)] != history:
        raise ValueError('XID_HISTORY_CHANGED_OR_LOST')
    return lines[len(history):]


def zero_delta(journal, history):
    """Windows and non-control native arms tolerate no fresh Xid."""
    if delta(journal, history):
        raise ValueError('NEW_HOST_XID')


def pin_binary(binary, product_revision):
    """Keep product revision distinct from the approved harness revision."""
    path = Path(binary)
    if (not re.fullmatch('[0-9a-f]{40}', product_revision)
            or path.parent.name != product_revision[:8]
            or path.name != 'qemu-system-x86_64'):
        raise ValueError('CANDIDATE_BINARY_PIN_MISMATCH')
    return product_revision
