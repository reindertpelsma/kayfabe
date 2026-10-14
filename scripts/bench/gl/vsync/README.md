# kf_glvsync — the swap-interval reproducer (Windows, WGL)

**STATUS: LIVE, 2026-10-14.** A native OpenGL app (no Java) that reproduces the Win11 vsync hang of
`traces/win11_opengl_20261014/`: `wglSwapIntervalEXT(--swap N)`, clear + triangle + SwapBuffers for
`--seconds`, windowed or `--fullscreen` (borderless popup); a watchdog prints `KFVS STALL` after 2 s without a
frame and `KFVS HANG` + exit 3 after `--hang-seconds`.

    x86_64-w64-mingw32-gcc -O2 -o kf_glvsync.exe kf_glvsync.c -lopengl32 -lgdi32 -luser32 -lwinmm
    kf_glvsync.exe --swap 1 --seconds 30 --hang-seconds 20 > vs.txt 2>&1

Run it in the interactive session (not as SYSTEM): `scripts/bench/windows/kfagent_ogl/runi.ps1 -Name vs -Cmd
"C:\kf\ogl\kf_glvsync.exe --swap 1 ..."`. Before `d86fea1f` (the vblank-count page): `KFVS HANG frame=1`.
After: ~60 fps, `KFVS RESULT OK`, windowed and fullscreen. The LWJGL/GLFW variant the owner asked for is not
written yet; the native one already reproduces the hang exactly.
