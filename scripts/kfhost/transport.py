"""Transport interface. The daemon core only sees Connection; tests inject a fake one."""

from . import VERSION


class Closed(Exception):
    """The peer closed (or the link dropped). code is the WebSocket close code or None."""

    def __init__(self, code=None):
        super().__init__("closed %s" % code)
        self.code = code


class Connection:
    def send(self, text):
        raise NotImplementedError

    def recv(self, timeout):
        """Return one text frame, or None when `timeout` seconds pass; raise Closed."""
        raise NotImplementedError

    def close(self):
        pass


class Transport:
    def connect(self, url, headers):
        raise NotImplementedError


class WebsocketsTransport(Transport):
    """Adapter for `websockets` >= 11 (sync client). NOT exercised without the package."""

    def connect(self, url, headers):
        from websockets.sync.client import connect  # imported lazily: the core is stdlib-only
        from websockets.exceptions import ConnectionClosed, WebSocketException

        try:
            sock = connect(url, additional_headers=headers, open_timeout=15, max_size=1 << 20, user_agent_header="kfhost/" + VERSION)
        except OSError as error:
            raise Closed(None) from error
        except WebSocketException as error:
            code = getattr(getattr(error, "response", None), "status_code", None)
            raise Closed(None if code is None else 4000 + code) from error

        class Conn(Connection):
            def send(self, text):
                try:
                    sock.send(text)
                except ConnectionClosed as error:
                    raise Closed(error.rcvd.code if error.rcvd else None) from error

            def recv(self, timeout):
                try:
                    data = sock.recv(timeout=timeout)
                except TimeoutError:
                    return None
                except ConnectionClosed as error:
                    raise Closed(error.rcvd.code if error.rcvd else None) from error
                return data if isinstance(data, str) else data.decode("utf-8", "replace")

            def close(self):
                sock.close()

        return Conn()
