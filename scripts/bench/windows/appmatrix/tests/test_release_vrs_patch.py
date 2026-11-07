# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Compile the actual VRS patch additions against CPU-only D3D enum/error stubs.

Falsifier: unsupported VRS returns normally, or tier 1/2 keeps 1x1 shading. No GPU API is called.
"""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import helpers as H


@unittest.skipUnless(shutil.which("c++"), "C++ compiler required")
class VrsPatch(unittest.TestCase):
    def run_fragment(self, tier, expect_throw):
        if shutil.disk_usage("/").free < 6 * 1024 ** 3:
            raise RuntimeError("J1 build safety stop: less than 6 GiB free")
        patch = (Path(H.AM) / "directx_vrs_gate.patch").read_text()
        body = "\n".join(line[1:] for line in patch.splitlines() if line.startswith("+") and not line.startswith("+++"))
        source = r'''#include <stdexcept>
constexpr int D3D12_VARIABLE_SHADING_RATE_TIER_1 = 1;
constexpr int D3D12_SHADING_RATE_2X2 = 5;
constexpr int E_NOTIMPL = -1;
void ThrowIfFailed(int hr) { if (hr < 0) throw std::runtime_error("unsupported"); }
struct Gate {
  int m_shadingRateTier;
  int m_refractionShadingRate = 0, m_sceneShadingRate = 0, m_postprocessShadingRate = 0;
  void apply() {
''' + body + "\n  }\n};\n"
        source += "int main(){Gate gate{%d};try{gate.apply();}catch(const std::runtime_error&){return %d;}return %s;}" % (
            tier, 0 if expect_throw else 1,
            "1" if expect_throw else "(gate.m_refractionShadingRate==5 && gate.m_sceneShadingRate==5 && gate.m_postprocessShadingRate==5) ? 0 : 1")
        with tempfile.TemporaryDirectory() as d:
            cpp, exe = Path(d) / "gate.cpp", Path(d) / "gate"
            cpp.write_text(source)
            p = subprocess.run(["c++", "-std=c++17", "-Wall", "-Werror", str(cpp), "-o", str(exe)], capture_output=True, text=True, timeout=30)
            self.assertEqual(p.returncode, 0, p.stderr)
            p = subprocess.run([str(exe)], capture_output=True, timeout=5)
            self.assertEqual(p.returncode, 0, "VRS patch did not enforce its CPU-model contract")

    def test_unsupported_vrs_cannot_reach_the_scene(self):
        self.run_fragment(0, True)

    def test_tier1_selects_coarse_shading(self):
        self.run_fragment(1, False)

    def test_tier2_selects_coarse_shading(self):
        self.run_fragment(2, False)


if __name__ == "__main__":
    unittest.main()
