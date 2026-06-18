@echo off
setlocal

rem Fluid launcher - single binary, single port (matches the release build).
rem Usage:  start.bat [PROJECT_DIR]   (default: sibling ..\alphaGPT)
rem The frontend is built into web\dist; the Rust binary serves it AND the API on
rem one port. Launch from this repo root so dotenvy reads Fluid\.env. Do not move.

set "ROOT=%~dp0"
if "%ROOT:~-1%"=="\" set "ROOT=%ROOT:~0,-1%"

set "PROJ=%~1"
if "%PROJ%"=="" set "PROJ=%ROOT%\..\alphaGPT"
for %%I in ("%PROJ%") do set "PROJ=%%~fI"

echo ============================================================
echo  Fluid launcher (single port)
echo    repo root : %ROOT%
echo    serving   : %PROJ%
echo    app + API : http://127.0.0.1:7878
echo ============================================================

if not exist "%PROJ%\" goto no_proj
if not exist "%ROOT%\.env" echo [!] WARNING: %ROOT%\.env not found - LLM disabled, cache/skeleton only.
if not exist "%ROOT%\web\node_modules\" goto npm_install
goto build_frontend

:npm_install
echo [*] Installing frontend deps (first run only)...
pushd "%ROOT%\web"
call npm install
popd
goto build_frontend

:build_frontend
echo [*] Building frontend into web\dist (so the binary serves it)...
pushd "%ROOT%\web"
call npm run build
popd
if errorlevel 1 goto build_failed

:launch
echo [*] Starting Fluid (first run compiles Rust - may take minutes)...
echo     Open http://127.0.0.1:7878   (use 127.0.0.1, not localhost)
echo     Close this window or press Ctrl+C to stop.
cd /d "%ROOT%"
cargo run --bin fluid -- "%PROJ%"
echo.
echo [!] Fluid exited (code %errorlevel%). If it quit immediately, port 7878 is
echo     likely held by another Fluid instance (e.g. a downloaded fluid-*.exe
echo     still running) - close that, then re-run start.bat.
pause
goto end

:build_failed
echo [X] frontend build failed - fix the error above, then re-run start.bat
pause
goto end

:no_proj
echo [X] project directory not found: %PROJ%
echo     usage: start.bat ^<PROJECT_DIR^>
pause

:end
endlocal
