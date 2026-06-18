@echo off
chcp 65001 >nul
setlocal EnableExtensions
title PlayBridge / PlayExtras Installer

rem Run this from the folder where MAA.exe is located.
rem MAA.exe가 있는 폴더에서 실행하세요.

set "ROOT=%~dp0"
set "EXE_DST=%ROOT%PlayBridgeADB.exe"
set "EXTRAS=%ROOT%PlayExtras"
set "DLL_DIR=%EXTRAS%\nx_device\15.0\shell\sdk"
set "DLL_DST=%DLL_DIR%\external_renderer_ipc.dll"

set "EXE_URL=https://github.com/HX3N/PlayBridge/releases/latest/download/PlayBridgeADB.exe"
set "DLL_URL=https://github.com/HX3N/PlayBridge/releases/latest/download/external_renderer_ipc.dll"

set "TMP_EXE=%TEMP%\PlayBridgeADB.download.exe"
set "TMP_DLL=%TEMP%\external_renderer_ipc.download.dll"

echo ============================================================
echo  PlayBridge / PlayExtras installer
echo ============================================================
echo.

rem --- MAA.exe 위치 확인 / Make sure this is the MAA folder ---
if not exist "%ROOT%MAA.exe" (
    echo [!] 이 폴더에서 MAA.exe를 찾을 수 없습니다.
    echo     MAA.exe가 있는 폴더에 이 파일을 두고 실행하세요.
    echo.
    echo [!] MAA.exe was not found in this folder.
    echo     Place this file in the folder where MAA.exe is located, then run it.
    echo.
    pause
    exit /b 1
)

rem --- 설치 전 설정 확인 / Confirm MAA settings before installing ---
echo ------------------------------------------------------------
echo  설치 전에 MAA 설정을 아래와 같이 맞춰 주세요:
echo.
echo    - 연결 프리셋                    : MuMu Player
echo    - 연결 주소                      : GooglePlayGames
echo    - ADB 경로                       : PlayBridgeADB.exe
echo    - MuMu 스크린샷 강화 기능 활성화 : 체크
echo    - MuMu 에뮬레이터 경로           : %EXTRAS% 또는 PlayExtras (상대 경로)
echo ------------------------------------------------------------
echo  Before installing, set your MAA settings as follows:
echo.
echo    - Connection preset                  : MuMu Player
echo    - Connection address                 : GooglePlayGames
echo    - ADB path                           : PlayBridgeADB.exe
echo    - Enable MuMu screenshot enhancement : ON
echo    - MuMu emulator path                 : %EXTRAS% or PlayExtras (rel path)
echo ------------------------------------------------------------
echo.
echo  위 설정을 마쳤으면 Enter를 눌러 설치를 시작하세요.
echo.
echo  Once the settings above are done, press Enter to start.
pause >nul
echo.

rem --- 점유 검사: 다운로드 전에 파일이 잠겨 있는지 먼저 확인 ---
rem --- Lock check: make sure the targets are free before downloading ---
:checklocks
set "LOCKED="
call :islocked "%EXE_DST%" || set "LOCKED=1"
call :islocked "%DLL_DST%" || set "LOCKED=1"
if defined LOCKED (
    echo [!] MAA가 실행 중이라 파일을 교체할 수 없습니다. MAA를 완전히 종료한 뒤 Enter를 누르세요.
    echo.
    echo [!] MAA is running and locking the files. Fully close MAA, then press Enter.
    pause >nul
    echo.
    goto checklocks
)

rem --- 다운로드 / Download ---
echo 최신 릴리스에서 다운로드 중...
echo.
echo Downloading the latest release...
echo.

curl.exe -L -f -o "%TMP_EXE%" "%EXE_URL%"
if errorlevel 1 (
    echo [X] PlayBridgeADB.exe 다운로드 실패. 인터넷 연결을 확인하세요.
    echo.
    echo [X] Failed to download PlayBridgeADB.exe. Check your internet connection.
    goto fail
)

curl.exe -L -f -o "%TMP_DLL%" "%DLL_URL%"
if errorlevel 1 (
    echo [X] external_renderer_ipc.dll 다운로드 실패. 최신 릴리스에 아직 포함되지 않았을 수 있습니다.
    echo.
    echo [X] Failed to download external_renderer_ipc.dll. It may not be in the latest release yet.
    goto fail
)

rem --- 배치 / Place files ---
mkdir "%DLL_DIR%" 2>nul

copy /Y "%TMP_EXE%" "%EXE_DST%" >nul
if errorlevel 1 (
    echo [X] PlayBridgeADB.exe 복사 실패 ^(파일 점유 가능성^). MAA를 종료하고 다시 실행하세요.
    echo.
    echo [X] Failed to copy PlayBridgeADB.exe ^(possibly in use^). Close MAA and run again.
    goto fail
)

copy /Y "%TMP_DLL%" "%DLL_DST%" >nul
if errorlevel 1 (
    echo [X] external_renderer_ipc.dll 복사 실패 ^(파일 점유 가능성^). MAA를 종료하고 다시 실행하세요.
    echo.
    echo [X] Failed to copy external_renderer_ipc.dll ^(possibly in use^). Close MAA and run again.
    goto fail
)

del "%TMP_EXE%" 2>nul
del "%TMP_DLL%" 2>nul

echo.
echo ============================================================
echo  [√] 설치 완료 / Installed
echo ============================================================
echo.
echo  배치 위치 / Placed at:
echo    %EXE_DST%
echo    %DLL_DST%
echo.
pause
exit /b 0

:fail
echo.
del "%TMP_EXE%" 2>nul
del "%TMP_DLL%" 2>nul
pause
exit /b 1

rem islocked "<file>"  ->  exit /b 1 if the file exists and is locked, else 0
:islocked
if not exist "%~1" exit /b 0
( call ) 1>>"%~1" 2>nul && exit /b 0
exit /b 1
