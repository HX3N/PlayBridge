@echo off
chcp 65001 >nul
setlocal EnableExtensions
title PlayBridge / PlayExtras Setup

rem Run this from the folder where MAA.exe is located.
rem MAA.exe가 있는 폴더에서 실행하세요.

rem This launcher only fetches and runs the latest installer engine, so the
rem install logic can change without users re-downloading this file.
set "ROOT=%~dp0"
set "ENGINE_URL=https://raw.githubusercontent.com/HX3N/PlayBridge/main/tools/Install.bat"
set "TMP_ENGINE=%TEMP%\PlayBridge.Install.download.bat"

echo ============================================================
echo  PlayBridge / PlayExtras setup
echo ============================================================
echo.
echo 최신 설치 스크립트를 받는 중...
echo.
echo Fetching the latest installer...
echo.

curl.exe -L -f -o "%TMP_ENGINE%" "%ENGINE_URL%"
if errorlevel 1 (
    echo [X] 설치 스크립트 다운로드 실패. 인터넷 연결을 확인하세요.
    echo.
    echo [X] Failed to download the installer. Check your internet connection.
    echo.
    pause
    exit /b 1
)

rem Pass this launcher's folder (the MAA folder) so the engine installs there
rem instead of the temp folder it was downloaded to.
call "%TMP_ENGINE%" "%ROOT%"
set "RC=%ERRORLEVEL%"

del "%TMP_ENGINE%" 2>nul
exit /b %RC%
