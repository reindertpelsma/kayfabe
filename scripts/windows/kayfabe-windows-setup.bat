@echo off
rem kayfabe-windows-setup.bat: runs kayfabe-windows-setup.ps1 (next to this file) with the execution policy it needs.
rem Right-click, "Run as administrator", or run it from an elevated prompt. Arguments go to the script:
rem   kayfabe-windows-setup.bat                    (phase auto)
rem   kayfabe-windows-setup.bat -Phase pre         (before the NVIDIA driver installer)
rem   kayfabe-windows-setup.bat -Phase post        (after it, then restart)
rem   kayfabe-windows-setup.bat -InstallDriver C:\nv\extracted
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0kayfabe-windows-setup.ps1" %*
set KF_SETUP_RC=%ERRORLEVEL%
rem Started by a double-click (cmd /c): keep the window open so the message can be read (set KF_SETUP_NOPAUSE=1 to never wait).
if not defined KF_SETUP_NOPAUSE echo %CMDCMDLINE% | find /i "/c" >nul && pause
exit /b %KF_SETUP_RC%
