@echo off
chcp 65001 1> NUL 2> NUL
setlocal enabledelayedexpansion
:MENU
cls

call :LOAD_ALL_CONFIGS

call :PRINT "1. SWIPE_SPEED" "%CURRENT_SWIPE_SPEED%"
call :PRINT "2. DEBUG" "%CURRENT_DEBUG%"
call :PRINT "3. DEBUG_CAPTURE" "%CURRENT_DEBUG_CAPTURE%"
echo 4. Reset
echo.
set /p CHOICE="Enter the number to modify: "

if "%CHOICE%"=="1" call :SET_VAR SWIPE_SPEED "%CURRENT_SWIPE_SPEED%" REG_DWORD
if "%CHOICE%"=="2" call :SET_VAR DEBUG "%CURRENT_DEBUG%" REG_DWORD
if "%CHOICE%"=="3" call :SET_VAR DEBUG_CAPTURE "%CURRENT_DEBUG_CAPTURE%" REG_DWORD
if "%CHOICE%"=="4" call :RESET_ALL
goto MENU

:PRINT
set "LABEL=%~1                  "
echo %LABEL:~0,20% %~2
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
)
goto MENU

:RESET_ALL
cls
echo Resetting all PlayBridge config registry values...
reg delete "HKCU\Software\PlayBridge\config" /v "SWIPE_SPEED" /f >nul 2>&1
reg delete "HKCU\Software\PlayBridge\config" /v "DEBUG" /f >nul 2>&1
reg delete "HKCU\Software\PlayBridge\config" /v "DEBUG_CAPTURE" /f >nul 2>&1

echo All PlayBridge config registry values have been deleted.

pause
goto MENU

:LOAD_ALL_CONFIGS
set "CURRENT_SWIPE_SPEED=10"
set "CURRENT_DEBUG=1"
set "CURRENT_DEBUG_CAPTURE=0"

for /f "skip=1 tokens=1,2,*" %%a in ('reg query "HKCU\Software\PlayBridge\config" 2^>nul') do (
  if "%%a"=="SWIPE_SPEED" call :HEX_TO_DEC "%%c" CURRENT_SWIPE_SPEED
  if "%%a"=="DEBUG" call :HEX_TO_DEC "%%c" CURRENT_DEBUG
  if "%%a"=="DEBUG_CAPTURE" call :HEX_TO_DEC "%%c" CURRENT_DEBUG_CAPTURE
)
exit /b

:HEX_TO_DEC
set "HEX_VAL=%~1"
set "RESULT_VAR=%~2"

echo "%HEX_VAL%" | findstr /c:"0x" >nul
if errorlevel 1 (
  set "%RESULT_VAR%=%HEX_VAL%"
  exit /b
)

set "HEX_VAL=%HEX_VAL:~2%"

for /f %%i in ('powershell -Command "[Convert]::ToInt32('%HEX_VAL%', 16)"') do set "%RESULT_VAR%=%%i"
exit /b
