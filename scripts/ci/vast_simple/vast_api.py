"""Vast console client for the simple CI runner (stdlib only).

CODE READ 2026-10-25: request shapes follow ci/vast-broker/src/vast.js (bundles query, PUT
asks create, inspect, instances-v1 listing). The key lives in memory and in one header; it is
never placed in a URL, a body, an argument vector or an error text.
"""

import json
import re
import urllib.error
import urllib.parse
import urllib.request

PREFIX = "kf-ci-simple-"
IMAGE = "docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16"
CONSOLE = "https://console.vast.ai"
LOOPBACK = re.compile(r"http://(127\.0\.0\.1|localhost|\[::1\]):[0-9]{1,5}")
MAX_BODY = 1 << 20
MAX_PAGES = 20


class ApiError(Exception):
    """Carries only a status, a fixed class and a short sanitised excerpt, never a credential."""


class Redactor:
    def __init__(self, *secrets):
        self.secrets = [s for s in secrets if s]

    def add(self, secret):
        if secret:
            self.secrets.append(secret)

    def clean(self, text):
        text = str(text)
        for secret in self.secrets:
            text = text.replace(secret, "***")
        return re.sub(r"(?i)bearer\s+\S+", "Bearer ***", text)

    def excerpt(self, text):
        """Plain characters only, 120 at most, dropped whole if it could carry a credential."""
        if not isinstance(text, str):
            return ""
        plain = re.sub(r"[^A-Za-z0-9 _.,:;'()/=-]", "", text)[:120]
        risky = any(re.search(r"(?i)bearer|[A-Za-z0-9_-]{24}", v) for v in (text, plain))
        return "" if risky or self.clean(text) != text else plain


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def valid_id(value):
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


class VastApi:
    def __init__(self, key, base=CONSOLE, timeout=20, redactor=None):
        if not isinstance(key, str) or not re.fullmatch(r"[\x21-\x7e]{8,256}", key):
            raise ApiError("no usable API key")
        if base != CONSOLE and not LOOPBACK.fullmatch(base):
            raise ApiError("API base must be https://console.vast.ai or loopback http")
        self.base, self.timeout = base, timeout
        self.redactor = redactor or Redactor()
        self.redactor.add(key)
        self.headers = {"Authorization": "Bearer " + key, "Content-Type": "application/json"}
        self.opener = urllib.request.build_opener(NoRedirect)

    def request(self, method, path, query=None, body=None, missing_ok=False):
        url = self.base + path + ("/" if not path.endswith("/") else "")
        if query:
            url += "?" + urllib.parse.urlencode(query)
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(url, data=data, method=method, headers=self.headers)
        try:
            with self.opener.open(req, timeout=self.timeout) as response:
                raw, status = response.read(MAX_BODY + 1), response.status
        except urllib.error.HTTPError as error:
            raw, status = error.read(MAX_BODY + 1), error.code
            error.close()
            if status == 404 and missing_ok:
                return None
        except (urllib.error.URLError, OSError, ValueError) as error:
            raise ApiError("transport error " + type(error).__name__) from None
        try:
            if len(raw) > MAX_BODY:
                raise ValueError
            parsed = json.loads(raw.decode("utf-8"))
        except ValueError:
            raise ApiError("HTTP %d unparsable" % status) from None
        if not 200 <= status < 300:
            text = parsed.get("error") or parsed.get("msg") if isinstance(parsed, dict) else ""
            raise ApiError("HTTP %d %s" % (status, self.redactor.excerpt(text)))
        return parsed

    def offers(self, cfg):
        query = {
            "limit": 100,
            "type": "ondemand",
            "gpu_arch": {"eq": "nvidia"},
            "compute_cap": {"gte": 750},
            "dph_total": {"lte": max(cfg["price"], cfg["sweepPrice"])},
            "vms_enabled": {"eq": True},
            "rentable": {"eq": True},
            "rented": {"eq": False},
            "num_gpus": {"eq": 1},
            "disk_space": {"gte": 120},
            "allocated_storage": 120,
            "inet_up_cost": {"lte": cfg["maxInetUsdPerGb"]},
            "inet_down_cost": {"lte": cfg["maxInetUsdPerGb"]},
            "order": [["dph_total", "asc"]],
        }
        result = self.request("POST", "/api/v0/bundles", body=query)
        offers = result.get("offers") if isinstance(result, dict) else None
        if not isinstance(offers, list) or len(offers) > 100:
            raise ApiError("malformed offers answer")
        return offers

    def create(self, offer_id, label):
        body = {
            "image": IMAGE,
            "vm": True,
            "disk": 120,
            "runtype": "ssh_direct",
            "onstart": "#!/bin/bash\ntrue\n",
            "env": {},
            "label": label,
            "target_state": "running",
            "cancel_unavail": True,
        }
        r = self.request("PUT", "/api/v0/asks/%d" % offer_id, body=body)
        if (
            not isinstance(r, dict)
            or r.get("success") is not True
            or not valid_id(r.get("new_contract"))
        ):
            raise ApiError("create refused")
        return r["new_contract"]

    def inspect(self, iid):
        r = self.request("GET", "/api/v0/instances/%d" % iid, missing_ok=True)
        if r is None or not isinstance(r, dict) or "instances" not in r:
            if r is None:
                return None
            raise ApiError("malformed inspect answer")
        inst = r["instances"]
        if inst is None:
            return None
        if not isinstance(inst, dict) or inst.get("id") != iid:
            raise ApiError("inspect answer is for another instance")
        return inst

    def list_instances(self, label=None):
        """instances-v1, strictly validated; a malformed page raises, it is never skipped."""
        rows, seen, token = [], set(), None
        for _ in range(MAX_PAGES):
            query = {"limit": 100}
            if label is not None:
                query["select_filters"] = json.dumps({"label": {"eq": label}})
            if token is not None:
                query["next_token"] = token
            r = self.request("GET", "/api/v1/instances", query=query)
            page = r.get("instances") if isinstance(r, dict) else None
            if (
                not isinstance(r, dict)
                or r.get("success") is not True
                or not isinstance(page, list)
            ):
                raise ApiError("malformed instance list")
            for row in page:
                lab = row.get("label") if isinstance(row, dict) else 0
                if not valid_id(row.get("id") if isinstance(row, dict) else None) or not (
                    lab is None or isinstance(lab, str)
                ):
                    raise ApiError("malformed instance row")
                if row["id"] in seen:
                    raise ApiError("duplicate instance row")
                seen.add(row["id"])
                rows.append(row)
            token = r.get("next_token")
            if token is None:
                return rows
            if not isinstance(token, str) or not token or len(token) > 512:
                raise ApiError("malformed page token")
        raise ApiError("too many pages")

    def destroy(self, iid, label=None):
        """DELETE one instance by id, only if its own record carries the runner's label prefix."""
        inst = self.inspect(iid)
        if inst is None:
            return False
        have = inst.get("label")
        if not isinstance(have, str) or not have.startswith(PREFIX) or label not in (None, have):
            raise ApiError("refusing to delete an instance that is not ours")
        r = self.request("DELETE", "/api/v0/instances/%d" % iid)
        if not isinstance(r, dict) or r.get("success") is not True:
            raise ApiError("delete not acknowledged")
        return True

    def boot_log(self, iid):
        """Best effort tail of the provider's console log; the signed URL is host-checked."""
        r = self.request("PUT", "/api/v0/instances/request_logs/%d" % iid, body={"tail": 40})
        url = r.get("result_url") if isinstance(r, dict) else None
        if r.get("success") is not True or not isinstance(url, str) or len(url) > 2048:
            return ""
        parts = urllib.parse.urlsplit(url)
        if (
            parts.scheme != "https"
            or parts.hostname != "s3.amazonaws.com"
            or parts.port
            or parts.username
            or not parts.path.startswith("/vast.ai/instance_logs/")
        ):
            return ""
        with self.opener.open(url, timeout=self.timeout) as response:
            return response.read(65536).decode("utf-8", "replace")
