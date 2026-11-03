"""Admission for one active box in this lane, on a shared account."""
def may_rent(live, owned_ids):
    owned={int(i) for i in owned_ids}
    return not any(int(row['id']) in owned for row in live)
