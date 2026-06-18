@echo off
chcp 65001 >nul
setlocal EnableExtensions

set "ROOT=%~dp0"
set "EXE_DST=%ROOT%PlayBridgeADB.exe"
set "EXTRAS=%ROOT%PlayExtras"
set "DLL_DIR=%EXTRAS%\nx_device\15.0\shell\sdk"
set "DLL_DST=%DLL_DIR%\external_renderer_ipc.dll"

set "SRC_EXE=%USERPROFILE%\Documents\GitHub\PlayBridge\target\release\PlayBridgeADB.exe"
set "SRC_DLL=%USERPROFILE%\Documents\GitHub\PlayBridge\target\release\external_renderer_ipc.dll"

if not exist "%ROOT%MAA.exe" (
    echo [!] MAA.exe not found here. Run from the MAA folder.
    pause & exit /b 1
)
if not exist "%SRC_EXE%" (
    echo [X] Not found: %SRC_EXE%
    pause & exit /b 1
)
if not exist "%SRC_DLL%" (
    echo [X] Not found: %SRC_DLL%
    pause & exit /b 1
)

:checklocks
set "LOCKED="
call :islocked "%EXE_DST%" || set "LOCKED=1"
call :islocked "%DLL_DST%" || set "LOCKED=1"
if defined LOCKED (
    echo [!] Files locked. Close MAA and press Enter.
    pause >nul & goto checklocks
)

mkdir "%DLL_DIR%" 2>nul
copy /Y "%SRC_EXE%" "%EXE_DST%" >nul || (echo [X] Copy failed: EXE & goto fail)
copy /Y "%SRC_DLL%" "%DLL_DST%" >nul || (echo [X] Copy failed: DLL & goto fail)

echo [OK] %EXE_DST%
echo [OK] %DLL_DST%
exit /b 0

:fail
pause & exit /b 1

:islocked
if not exist "%~1" exit /b 0
( call ) 1>>"%~1" 2>nul && exit /b 0
exit /b 1
