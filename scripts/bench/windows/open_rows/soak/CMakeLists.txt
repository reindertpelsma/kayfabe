# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Builds kf_gldual.exe (the GL dual-context soak of the open Windows rows) with the Visual Studio generator, so that no vcvars shell is needed.
# STATUS: DESIGN-ONLY, 2026-11-17; never configured on Windows.
cmake_minimum_required(VERSION 3.20)
project(kf_gldual C)
add_executable(kf_gldual ../kf_gldual.c)
target_link_libraries(kf_gldual PRIVATE opengl32 user32 gdi32)
