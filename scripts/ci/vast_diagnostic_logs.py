"""Read only the provider-returned S3 log for one explicitly owned instance.

This diagnostic-only exception is separate from the Worker's fixed-host rules.
The caller authenticates the request_logs API request and supplies its exact response.
Returned bytes are untrusted: scrub them before displaying or persisting public evidence.
Neither URLs nor downloaded contents are logged here. No Vast SDK fallback is used.
"""

import re
import urllib.parse
import urllib.request

MAX_BYTES = 5_000_000
TIMEOUT_SECONDS = 30


class LogRefused(ValueError):
    """A diagnostic failed without exposing its signed URL or remote content."""


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def is_vast_api_url(url):
    """The normal API path retains its HTTPS Vast-only rule."""
    try:
        parsed = urllib.parse.urlsplit(url)
        return (
            parsed.scheme == "https"
            and parsed.hostname is not None
            and (parsed.hostname == "vast.ai" or parsed.hostname.endswith(".vast.ai"))
            and not parsed.username
            and not parsed.password
            and parsed.port in (None, 443)
            and not parsed.fragment
        )
    except (TypeError, ValueError):
        return False


def fetch_instance_log(owned_id, requested_id, api_response, *, opener=None):
    """Download exactly result_url from request_logs for the caller's owned id.

    This function cannot select another response field or accept a replacement URL.
    Its fresh request contains no account Authorization or Cookie headers. The default
    opener has no cookie processor, and refuses every redirect, including S3 redirects.
    """
    if type(owned_id) is not int or owned_id <= 0 or owned_id != requested_id:
        raise LogRefused("owned instance mismatch")
    try:
        url = api_response["result_url"]
        parsed = urllib.parse.urlsplit(url)
        hostname = parsed.hostname or ""
        s3_host = re.fullmatch(r"(?:[a-z0-9-]+\.)*s3(?:[.-][a-z0-9-]+)*\.amazonaws\.com", hostname)
        if (
            parsed.scheme != "https"
            or not s3_host
            or parsed.username
            or parsed.password
            or parsed.port not in (None, 443)
            or parsed.fragment
        ):
            raise ValueError("invalid diagnostic target")
    except (KeyError, TypeError, ValueError):
        raise LogRefused("invalid diagnostic target") from None
    try:
        if opener is None:
            opener = urllib.request.build_opener(NoRedirect())
        request = urllib.request.Request(url, headers={}, method="GET")
        with opener.open(request, timeout=TIMEOUT_SECONDS) as response:
            raw = response.read(MAX_BYTES + 1)
            if response.status != 200 or len(raw) > MAX_BYTES:
                raise ValueError("invalid diagnostic response")
        return raw
    except Exception:
        raise LogRefused("diagnostic download refused") from None
