@echo off
rem SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
rem Run in VS 2022 x64 Native Tools prompt, with the Windows SDK + WDK installed.
setlocal
cd /d "%~dp0"
msbuild gsptrace.vcxproj /p:Configuration=Release /p:Platform=x64 /m:1 /v:m
if errorlevel 1 exit /b 1
cl /nologo /W4 /WX /O2 /D_CRT_SECURE_NO_WARNINGS collect.c advapi32.lib /Fe:build\gsptrace.exe /Fo:build\collect.obj
if errorlevel 1 exit /b 1
cl /nologo /W4 /WX /O2 tests\windows_api_test.c advapi32.lib /Fe:build\windows_api_test.exe /Fo:build\windows_api_test.obj
if errorlevel 1 exit /b 1
cl /nologo /W4 /WX /I. queue.c tests\queue_test.c /Fe:build\queue_test.exe /Fo:build\
if errorlevel 1 exit /b 1
build\queue_test.exe
exit /b %ERRORLEVEL%
