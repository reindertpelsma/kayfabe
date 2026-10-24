"""SW3 offer admission predicates extracted from the original operator."""


class OfferRefused(ValueError):
    """This offer cannot be allocated by SW3."""


def require_offer(offer, blacklisted):
    if offer.get("vms_enabled") is not True or offer.get("num_gpus") != 1:
        raise OfferRefused("one VM-capable GPU required")
    if offer.get("is_vm_deverified") is not False:
        raise OfferRefused("VM status is unavailable or disqualified")
    if offer.get("machine_id") in blacklisted:
        raise OfferRefused("blacklisted machine")
    if not 0 < float(offer.get("dph_total", 0)) <= 2.50:
        raise OfferRefused("hourly price outside lane limit")
    return offer


def eligible(offers, blacklisted):
    accepted = []
    for offer in offers:
        try:
            accepted.append(require_offer(offer, blacklisted))
        except OfferRefused:
            continue
    return accepted
