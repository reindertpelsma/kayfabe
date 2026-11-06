"""SW4 admission retains SW3 guards and pins the independent grant."""

import math


class OfferRefused(ValueError):
    """This offer cannot be allocated by SW3."""


def require_offer(offer, blacklisted):
    if offer.get("verification") != "verified":
        raise OfferRefused("verified offers only; OWNER_RULINGS T.12")
    if offer.get("vms_enabled") is not True or offer.get("num_gpus") != 1:
        raise OfferRefused("one VM-capable GPU required")
    if offer.get("is_vm_deverified") is not False:
        raise OfferRefused("VM status is unavailable or disqualified")
    if offer.get("machine_id") in blacklisted or offer.get("host_id") in blacklisted:
        raise OfferRefused("blacklisted machine")
    pairs = {57427095: 99805, 50694293: 45428}
    if pairs.get(offer.get("id")) != offer.get("machine_id") or offer.get("host_id") != 70221:
        raise OfferRefused("candidate outside SW4 grant")
    location = offer.get("geolocation")
    if not isinstance(location, str) or not location.endswith(", CA"):
        raise OfferRefused("candidate country is not granted Canada")
    rate = offer.get("dph_total")
    if type(rate) not in (int, float) or not math.isfinite(rate) or not 0 < rate <= 0.60:
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
