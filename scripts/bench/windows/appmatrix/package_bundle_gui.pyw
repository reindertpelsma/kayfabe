#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Optional GUI for the same shell-free builder; uses CPython's bundled Tk.

Double-click after installing official CPython with Tcl/Tk support. No commands
are typed or passed to a shell. Manual downloads use the paths shown by Layout;
Verify checks their sizes and SHA-256 pins without downloading or extracting apps.
"""
import contextlib
from pathlib import Path
import queue
import sys
import threading
import tkinter as tk
from tkinter import filedialog, ttk

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import package_bundle


def main():
    root = tk.Tk()
    root.title("KFAPPS reproducible bundle")
    root.geometry("900x560")
    fields = {}
    for row, (label, default, directory) in enumerate((
        ("Manifest", HERE / "manifest.json", False),
        ("Profiles", HERE / "gate_profiles.json", False),
        ("Downloads", Path.cwd() / "out" / "downloads", True),
        ("Output", Path.cwd() / "out", True))):
        value = tk.StringVar(value=str(default)); fields[label] = value
        ttk.Label(root, text=label).grid(row=row, column=0, sticky="w", padx=8, pady=4)
        ttk.Entry(root, textvariable=value).grid(row=row, column=1, sticky="ew")
        def browse(v=value, d=directory):
            selected = filedialog.askdirectory() if d else filedialog.askopenfilename(filetypes=[("JSON", "*.json")])
            if selected:
                v.set(selected)
        ttk.Button(root, text="Browse", command=browse).grid(row=row, column=2, padx=8)
    ttk.Label(root, text="Check → Fetch (or copy official downloads manually) → Verify → Build → Verify image").grid(row=4, column=0, columnspan=3, sticky="w", padx=8, pady=8)
    controls = ttk.Frame(root); controls.grid(row=5, column=0, columnspan=3, sticky="w", padx=8)
    output = tk.Text(root, wrap="word", state="disabled")
    output.grid(row=6, column=0, columnspan=3, sticky="nsew", padx=8, pady=8)
    root.columnconfigure(1, weight=1); root.rowconfigure(6, weight=1)
    messages = queue.Queue()
    buttons = []
    active = False
    class Sink:
        def write(self, message):
            messages.put(message)
            return len(message)
        def flush(self):
            pass
    def start(command, extra=()):
        nonlocal active
        active = True
        for b in buttons:
            b.configure(state="disabled")
        args = [command, "--manifest", fields["Manifest"].get(), "--profiles", fields["Profiles"].get(),
                "--dl", fields["Downloads"].get(), "--out", fields["Output"].get()]
        args.extend(extra)
        def run():
            try:
                with contextlib.redirect_stdout(Sink()), contextlib.redirect_stderr(Sink()):
                    rc = package_bundle.main(args)
                messages.put(f"Finished: exit {rc}\n")
            except Exception as error:
                messages.put(f"FAIL {error}\n")
            finally:
                messages.put(None)
        threading.Thread(target=run, daemon=False).start()
    for title, command in (("Check", "check"), ("Layout", "layout"), ("Fetch", "fetch"), ("Verify", "verify"), ("Build", "build"), ("Verify image", "verify-image")):
        button = ttk.Button(controls, text=title, command=lambda c=command: start(c))
        button.pack(side="left", padx=3); buttons.append(button)
    def compare():
        receipt = filedialog.askopenfilename(title="Choose expected manifest from first build", filetypes=[("JSON", "*.json")])
        if not receipt:
            return
        iso = filedialog.askopenfilename(title="Choose ISO from second build", filetypes=[("ISO", "*.iso")])
        if iso:
            start("verify-image", ["--iso", iso, "--receipt", receipt])
    button = ttk.Button(controls, text="Compare receipt", command=compare)
    button.pack(side="left", padx=3); buttons.append(button)
    def drain():
        nonlocal active
        while not messages.empty():
            message = messages.get_nowait()
            if message is None:
                active = False
                for b in buttons:
                    b.configure(state="normal")
            else:
                output.configure(state="normal"); output.insert("end", message); output.see("end"); output.configure(state="disabled")
        root.after(100, drain)
    # Keep the window open during a build, so closing it cannot orphan a compiler.
    root.protocol("WM_DELETE_WINDOW", lambda: None if active else root.destroy())
    root.after(100, drain)
    root.mainloop()


if __name__ == "__main__":
    main()
