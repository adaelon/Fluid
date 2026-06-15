@echo off
setlocal

rem Fluid dev launcher - backend (Rust :7878) + frontend (Vite :5173)
rem Usage:  start.bat [PROJECT_DIR]   (default: sibling ..\alphaGPT)
rem Backend launches from this repo root so dotenvy reads Fluid\.env. Do not move.

set "ROOT=%~dp0"
if "%ROOT:~-1%"=="\" set "ROOT=%ROOT:~0,-1%"

set "PROJ=%~1"
if "%PROJ%"=="" set "PROJ=%ROOT%\..\alphaGPT"
for %%I in ("%PROJ%") do set "PROJ=%%~fI"

echo ============================================================
echo  Fluid dev launcher
echo    repo root : %ROOT%
echo    serving   : %PROJ%
echo    backend   : http://127.0.0.1:7878
echo    frontend  : http://127.0.0.1:5173
echo ============================================================

if not exist "%PROJ%\" goto no_proj
if not exist "%ROOT%\.env" echo [!] WARNING: %ROOT%\.env not found - LLM disabled, cache/skeleton only.
if not exist "%ROOT%\web\node_modules\" goto npm_install
goto launch

:npm_install
echo [*] Installing frontend deps (first run only)...
pushd "%ROOT%\web"
call npm install
popd
goto launch

:launch
echo [*] Starting backend (first run compiles Rust - may take minutes)...
start "Fluid Backend :7878" /D "%ROOT%" cmd /k cargo run --bin fluid -- "%PROJ%"
echo [*] Starting frontend...
start "Fluid Frontend :5173" /D "%ROOT%\web" cmd /k npm run dev
echo.
echo Both services launched in separate windows.
echo Open:  http://127.0.0.1:5173    (use 127.0.0.1, not localhost)
goto end

:no_proj
echo [X] project directory not found: %PROJ%
echo     usage: start.bat ^<PROJECT_DIR^>
pause

:end
endlocal
