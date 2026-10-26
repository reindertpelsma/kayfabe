#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Sign in a baseline Windows guest through QMP; verify the session through QGA.

Usage: signin.py QMP_SOCKET QGA_SOCKET PASSWORD_FILE
The rental-only kf account password is read from PASSWORD_FILE, never printed or put in argv. The caller
sets that password separately and holds the GPU lock. Assumes the guest's US keyboard.
"""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import time


def keycodes(password):
    punctuation = {
        " ": "spc", "-": "minus", "=": "equal", "[": "bracket_left",
        "]": "bracket_right", ";": "semicolon", "'": "apostrophe",
        ",": "comma", ".": "dot", "/": "slash", "\\": "backslash", "`": "grave_accent",
    }
    shifted = dict(zip('!@#$%^&*()_+{}:"<>?|~', '1234567890-=[];\',./\\`'))
    result = []
    for char in password:
        shift = char in shifted or "A" <= char <= "Z"
        base = shifted.get(char, char.lower())
        if "a" <= base <= "z" or "0" <= base <= "9":
            code = base
        elif base in punctuation:
            code = punctuation[base]
        else:
            raise ValueError("password requires a character unsupported by the US QMP keyboard")
        result.append((["shift"] if shift else []) + [code])
    return result


def main():
    if len(sys.argv) != 4:
        raise ValueError("usage: signin.py QMP_SOCKET QGA_SOCKET PASSWORD_FILE")
    password = Path(sys.argv[3]).read_text().strip()
    if not password:
        raise ValueError("the per-VM password file must be nonempty")
    chords = keycodes(password)  # Validate everything before sending any input.
    helper = Path("/root/kayfabe/scripts/bench/windows/qmp.py")

    def session():
        r = subprocess.run(
            [sys.executable, str(helper), sys.argv[2], "qga-exec", "powershell.exe",
             "-NoProfile", "-Command", "$u=(Get-CimInstance Win32_ComputerSystem).UserName; "
             "if ($u -and -not (Get-Process LogonUI -ErrorAction SilentlyContinue) -and (Get-Process explorer -ErrorAction SilentlyContinue)) { $u }"],
            capture_output=True, text=True, timeout=20, check=False,
        )
        return r.returncode == 0 and r.stdout.strip().lower().endswith("\\kf")

    if session():
        print("SIGNIN_VERIFIED already signed in")
        return
    with socket.socket(socket.AF_UNIX) as sock:
        sock.settimeout(15)
        sock.connect(sys.argv[1])
        f = sock.makefile("rwb", buffering=0)
        json.loads(f.readline())
        seq = 0

        def command(name, args=None):
            nonlocal seq
            seq += 1
            msg = {"execute": name, "id": seq}
            if args is not None:
                msg["arguments"] = args
            f.write(json.dumps(msg).encode() + b"\n")
            while True:
                reply = json.loads(f.readline())
                if reply.get("id") == seq:
                    if "error" in reply:
                        raise RuntimeError("QMP rejected sign-in input")
                    return

        def key(chord):
            command("send-key", {"keys": [{"type": "qcode", "data": k} for k in chord], "hold-time": 50})
            time.sleep(0.12)

        command("qmp_capabilities")
        key(["spc"])
        time.sleep(2)
        key(["ctrl", "a"])
        key(["backspace"])
        for chord in chords:
            key(chord)
        key(["ret"])
    for _ in range(30):
        if session():
            print("SIGNIN_VERIFIED interactive user is present")
            return
        time.sleep(1)
    raise RuntimeError("sign-in was not verified; interactive probes must not run")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, RuntimeError, OSError, subprocess.TimeoutExpired):
        print("SIGNIN_FAILED: check account, session, and keyboard configuration", file=sys.stderr)
        sys.exit(1)
