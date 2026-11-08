# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Bounded QEMU socket sinks. Guest serial output never writes unbounded files."""

import os
from pathlib import Path
import selectors
import socket
import tempfile


class Channels:
    def __init__(self, directory, limits):
        self.root = Path(directory)
        self.socket_dir = tempfile.TemporaryDirectory(prefix="kf-image-ch-")
        self.socket_root = Path(self.socket_dir.name)
        self.selector = selectors.DefaultSelector()
        self.listeners = []
        self.streams = {}
        self.files = {}
        self.counts = {}
        self.limits = limits
        try:
            for name in limits:
                path = self.path(name)
                listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                listener.bind(str(path))
                listener.listen(1)
                listener.setblocking(False)
                self.listeners.append(listener)
                self.selector.register(listener, selectors.EVENT_READ, ("listen", name))
                self.files[name] = (
                    self.root
                    / (
                        name + ".log"
                        if name == "serial"
                        else name + ".json" if name == "inventory" else name + ".tar.gz"
                    )
                ).open("wb")
                self.counts[name] = 0
        except BaseException:
            self.close()
            raise

    def path(self, name):
        return self.socket_root / (name + ".sock")

    def add_pipe(self, stream, name, limit):
        os.set_blocking(stream.fileno(), False)
        self.limits[name] = limit
        self.streams[name] = stream
        self.files[name] = (self.root / (name + ".log")).open("wb")
        self.counts[name] = 0
        self.selector.register(stream, selectors.EVENT_READ, ("pipe", name))

    def pump(self, timeout=0.1):
        for key, _ in self.selector.select(timeout):
            kind, name = key.data
            if kind == "listen":
                stream, _ = key.fileobj.accept()
                stream.setblocking(False)
                self.selector.unregister(key.fileobj)
                key.fileobj.close()
                self.streams[name] = stream
                self.selector.register(stream, selectors.EVENT_READ, ("data", name))
            else:
                try:
                    size = min(65536, self.limits[name] - self.counts[name] + 1)
                    block = (
                        os.read(key.fd, size)
                        if kind == "pipe"
                        else key.fileobj.recv(size)
                    )
                except BlockingIOError:
                    continue
                if not block:
                    self.selector.unregister(key.fileobj)
                    key.fileobj.close()
                    continue
                if self.counts[name] + len(block) > self.limits[name]:
                    raise ValueError("guest " + name + " output limit exceeded")
                self.files[name].write(block)
                self.files[name].flush()
                self.counts[name] += len(block)

    def drain(self):
        for _ in range(10000):
            if not self.selector.select(0):
                break
            self.pump(0)

    def close(self):
        for stream in list(self.streams.values()) + self.listeners:
            try:
                stream.close()
            except OSError:
                pass
        for f in self.files.values():
            f.close()
        self.selector.close()
        for name in self.limits:
            try:
                self.path(name).unlink(missing_ok=True)
            except OSError:
                pass
        self.socket_dir.cleanup()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()
