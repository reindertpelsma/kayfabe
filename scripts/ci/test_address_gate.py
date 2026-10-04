# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The address gate (G1/G1b/G1c) can fail: its fixtures, and the shapes it was built from."""

import unittest

import address_gate as g


class AddressGate(unittest.TestCase):
    def test_every_known_positive_is_found_and_no_look_alike_flagged(self):
        self.assertEqual(g.self_test(), [])

    def test_the_lexer_drops_comments_and_strings_but_keeps_lines(self):
        code, strings = g.lex('let a = "x.as_ptr()"; // b.as_ptr()\n/* c.as_ptr() */ let d = 1;\n')
        self.assertEqual(len(code), 3)
        self.assertNotIn("as_ptr", "".join(code))
        self.assertIn("x.as_ptr()", strings[0])

    def test_a_lifetime_is_not_a_char_literal(self):
        code, _ = g.lex("fn f<'a>(x: &'a u8) -> *const u8 { x }\n")
        self.assertIn("*const", code[0])

    def test_the_pre_change_shapes_fire(self):
        # e4fb0190: kf-cuda's console frame address, kf-qemu's FrameView, kf-util's key.
        for line in [
            "    pub fn addr(&self) -> usize {",
            "    pub addr: usize,",
            "    addr: AtomicUsize,",
            "        let key = std::ptr::from_ref(site) as usize;",
            "pub type CUdeviceptr = u64;",
        ]:
            self.assertTrue(g.g1_hits(line + "\n"), line)

    def test_a_derived_debug_over_a_pointer_fires_and_a_manual_one_does_not(self):
        self.assertTrue(g.g1b_hits("#[derive(Debug)]\npub struct HostSpan {\n    base: NonNull<u8>,\n}\n"))
        self.assertFalse(g.g1b_hits("pub struct HostSpan {\n    base: NonNull<u8>,\n}\n"))

    def test_the_review_shapes_fire_statement_by_statement(self):
        # review of v3-sec-rawaddr (2026-10-04): each passed G1 line by line
        for src in [
            'let s = format!(concat!("{:",\n"p}"), r);\n',
            'let s = format!(concat!("{:", stringify!(p), "}"), r);\n',
            "use core::fmt::*;\nfn f() { <&u8 as Pointer>::fmt(&r, f); }\n",
            "use core::ptr::{hash};\nfn f() { hash(r, h); }\n",
            "use core::ptr::{\n    hash,\n};\n",
        ]:
            self.assertTrue(g.g1_hits(src), src)
        # and the tree's own look-alikes stay quiet
        for src in [
            'write!(f, concat!(stringify!(Gpa), "({:#x})"), self.0)\n',
            "use std::hash::{Hash, Hasher};\nuse core::fmt::{self, Write};\n",
            'let a = "{:"; \n let b = 1;\n',
        ]:
            self.assertFalse(g.g1_hits(src), src)

    def test_a_derived_debug_over_a_driver_handle_fires(self):
        for body in ["graph: usize,", "exec: usize,", "handle: u64,", "addr: DevAddr,", "userspace_addr: u64,"]:
            self.assertTrue(g.g1b_hits(f"#[derive(Debug)]\nstruct S {{\n    {body}\n}}\n"), body)
        # kf-cuda's stricter rule: a CUDA address under any `…addr` name; a guest address elsewhere may print
        va = "#[derive(Debug)]\nstruct VaReservation {\n    addr: u64,\n}\n"
        self.assertTrue(g.g1b_hits(va, cuda=True))
        self.assertFalse(g.g1b_hits(va))

    def test_a_pointer_reexport_fires(self):
        self.assertTrue(g.g1c_hits("pub use core::ptr::NonNull as Handle;\n"))
        self.assertFalse(g.g1c_hits("use core::ptr::NonNull;\n"))


if __name__ == "__main__":
    unittest.main()
