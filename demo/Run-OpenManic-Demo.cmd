@echo off
setlocal
REM OpenManic demo launcher: loads the bundled sample database.
REM The store_id inside the sample DB is bound to this exact data path,
REM so the demo must always run against C:\Users\Public\OpenManicDemo.
set "OPENMANIC_DATA_DIR=C:\Users\Public\OpenManicDemo"

if not exist "%~dp0OpenManic.exe" (
  echo Copy OpenManic.exe into this folder ^(next to this script^), then run this again.
  pause
  exit /b 1
)

if not exist "%OPENMANIC_DATA_DIR%" mkdir "%OPENMANIC_DATA_DIR%"
if not exist "%OPENMANIC_DATA_DIR%\openmanic.sqlite3" (
  copy /y "%~dp0demo-data\openmanic.sqlite3" "%OPENMANIC_DATA_DIR%\openmanic.sqlite3" >nul
  echo Installed sample database into %OPENMANIC_DATA_DIR%
)

start "" "%~dp0OpenManic.exe"
endlocal
