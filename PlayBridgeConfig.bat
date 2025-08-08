@echo off
chcp 65001 1> NUL 2> NUL
setlocal
:MENU
cls

set "CURRENT_TITLE="
for /f "tokens=3*" %%a in ('reg query "HKCU\Software\PlayBridge\config" /v "TITLE" 2^>nul') do set "CURRENT_TITLE=%%a"
if not defined CURRENT_TITLE set "CURRENT_TITLE=명일방주"
call :PRINT "1. TITLE" "%CURRENT_TITLE%"

set "CURRENT_PACKAGE="
for /f "tokens=3*" %%a in ('reg query "HKCU\Software\PlayBridge\config" /v "PACKAGE" 2^>nul') do set "CURRENT_PACKAGE=%%a"
if not defined CURRENT_PACKAGE set "CURRENT_PACKAGE=com.YoStarKR.Arknights"
call :PRINT "2. PACKAGE" "%CURRENT_PACKAGE%"

set "CURRENT_SWIPE_SPEED="
for /f "tokens=3*" %%a in ('reg query "HKCU\Software\PlayBridge\config" /v "SWIPE_SPEED" 2^>nul') do set "CURRENT_SWIPE_SPEED=%%a"
if not defined CURRENT_SWIPE_SPEED set "CURRENT_SWIPE_SPEED=10"
call :PRINT "3. SWIPE_SPEED" "%CURRENT_SWIPE_SPEED%"

set "CURRENT_DEBUG="
for /f "tokens=3*" %%a in ('reg query "HKCU\Software\PlayBridge\config" /v "DEBUG" 2^>nul') do set "CURRENT_DEBUG=%%a"
if not defined CURRENT_DEBUG set "CURRENT_DEBUG=0"
call :PRINT "4. DEBUG" "%CURRENT_DEBUG%"

set "CURRENT_DEBUG_CAPTURE="
for /f "tokens=3*" %%a in ('reg query "HKCU\Software\PlayBridge\config" /v "DEBUG_CAPTURE" 2^>nul') do set "CURRENT_DEBUG_CAPTURE=%%a"
if not defined CURRENT_DEBUG_CAPTURE set "CURRENT_DEBUG_CAPTURE=0"
call :PRINT "5. DEBUG_CAPTURE" "%CURRENT_DEBUG_CAPTURE%"
echo 6. Reset
echo.
set /p CHOICE="Enter the number to modify: "

if "%CHOICE%"=="1" call :SET_VAR TITLE "%CURRENT_TITLE%" REG_SZ
if "%CHOICE%"=="2" call :SET_VAR PACKAGE "%CURRENT_PACKAGE%" REG_SZ
if "%CHOICE%"=="3" call :SET_VAR SWIPE_SPEED "%CURRENT_SWIPE_SPEED%" REG_DWORD
if "%CHOICE%"=="4" call :SET_VAR DEBUG "%CURRENT_DEBUG%" REG_DWORD
if "%CHOICE%"=="5" call :SET_VAR DEBUG_CAPTURE "%CURRENT_DEBUG_CAPTURE%" REG_DWORD
if "%CHOICE%"=="6" call :RESET_ALL
goto MENU

:PRINT
set "LABEL=%~1"
set "VALUE=%~2"
set "LABEL=%LABEL%                  "
set "LABEL=%LABEL:~0,20%"
echo %LABEL% %VALUE%
exit /b

:SET_VAR
cls
set VAR_NAME=%1
set CURRENT_VALUE=%2
set REG_TYPE=%3

set /p NEW_VALUE="%VAR_NAME% (current: %CURRENT_VALUE%): "
if not "%NEW_VALUE%"=="" (
  reg add "HKCU\Software\PlayBridge\config" /v "%VAR_NAME%" /t %REG_TYPE% /d "%NEW_VALUE%" /f >nul
  echo %VAR_NAME% has been set to %NEW_VALUE%.
) else (
  goto SET_VAR
)
pause
goto MENU

:RESET_ALL
cls
echo Resetting all PlayBridge config registry values...
reg delete "HKCU\Software\PlayBridge\config" /v "TITLE" /f >nul 2>&1
reg delete "HKCU\Software\PlayBridge\config" /v "PACKAGE" /f >nul 2>&1
reg delete "HKCU\Software\PlayBridge\config" /v "SWIPE_SPEED" /f >nul 2>&1
reg delete "HKCU\Software\PlayBridge\config" /v "DEBUG" /f >nul 2>&1
reg delete "HKCU\Software\PlayBridge\config" /v "DEBUG_CAPTURE" /f >nul 2>&1

echo All PlayBridge config registry values have been deleted.

pause
goto MENU
