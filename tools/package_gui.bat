@echo off
REM ============================================================================
REM  K music (musicMT) one-click packaging script  (dist builder)
REM
REM  Usage: double-click this file, or run  tools\package_gui.bat
REM
REM  WHAT IT DOES
REM    [1/2] cargo build --release -p mt-gui -p mt-cli
REM    [2/2] copy target\release\mt-gui.exe -> dist\k_music.exe (GUI entry)
REM          copy target\release\mt-cli.exe -> dist\ + README
REM
REM  The frontend static assets (index.html / styles.css / app.js) are copied
REM    automatically by app/gui/build.rs at compile time into dist\frontend,
REM    so this script does NOT need to copy them.
REM
REM  *** RUNTIME CONFIG FILES ARE NEVER DELETED OR OVERWRITTEN ***
REM    Everything under dist\ that is produced at runtime is preserved:
REM    dist\kqk-app.toml, dist\.guest\, dist\library.db, dist\app.log ...
REM    These hold the user's test config (current library path, NCM tool path,
REM    import source, UI preferences). They MUST survive every rebuild so the
REM    user does not have to re-select the library each time.
REM    This script overwrites k_music.exe / mt-cli.exe / README.md, and removes
REM    the obsolete dist\mt-gui.exe left by older builds. Runtime config stays.
REM
REM  *** WRITING RULES FOR THIS FILE (learned the hard way - do not remove) ***
REM    1) CRLF line endings. With LF-only, cmd.exe mis-parses the file and
REM       splits lines into bogus commands ("xxx is not recognized ...").
REM    2) ASCII only. Never put non-ASCII text in a .bat file. This machine's
REM       console code page is 936 (GBK) but a GBK-encoded .bat breaks under
REM       code page 65001, and a UTF-8 .bat breaks under 936 - cmd.exe
REM       mis-decodes multi-byte chars and loses byte alignment, which
REM       corrupts the parse. ASCII is the only encoding safe everywhere.
REM       All Chinese documentation lives in tools\dist_readme.md (UTF-8),
REM       which is copied to dist\README.md.
REM    3) No backticks - the backtick is cmd.exe's escape character.
REM    4) Do not switch code pages (chcp) inside the script - cmd re-reads the
REM       file with the new code page and the byte offset goes out of sync.
REM ============================================================================
setlocal
cd /d "%~dp0.."

echo [1/2] Building release (mt-gui + mt-cli) ...
cargo build --release -p mt-gui -p mt-cli
if errorlevel 1 (
  echo [ERR] Build failed. Make sure the Rust toolchain 'cargo' is installed
  echo       and that you are running this script from the project root.
  exit /b 1
)

echo [2/2] Copying exe / README into dist\ ...
if not exist dist mkdir dist
copy /Y target\release\mt-gui.exe dist\k_music.exe >nul
if errorlevel 1 (
  echo [ERR] Cannot update dist\k_music.exe - close K music and retry.
  exit /b 1
)
copy /Y target\release\mt-cli.exe dist\ >nul
if errorlevel 1 (
  echo [ERR] Cannot update dist\mt-cli.exe - close K music and retry.
  exit /b 1
)
if exist dist\mt-gui.exe del /f /q dist\mt-gui.exe >nul 2>&1
if exist tools\dist_readme.md copy /Y tools\dist_readme.md dist\README.md >nul

echo Done.
echo.
echo  Publish folder: dist\
echo     k_music.exe   GUI entry - double-click it, no CLI needed
echo     mt-cli.exe    CLI tool (scan / import / serve / import-netease / ncm-organize)
echo     frontend\     Frontend assets - synced automatically by build.rs
echo     README.md    Full Chinese documentation
echo.
echo  Preserved runtime config (never deleted or overwritten by this build):
if exist dist\kqk-app.toml echo     dist\kqk-app.toml
if exist dist\.guest echo     dist\.guest\
if exist dist\library.db echo     dist\library.db
goto :eof
