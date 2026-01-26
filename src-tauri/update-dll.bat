@echo off
echo Copying latest version.dll to Tauri resources...

set SOURCE=..\version-dll\build\Release\version.dll
set TARGET=resources\version.dll

if not exist "%SOURCE%" (
    echo Error: Source DLL not found at %SOURCE%
    echo Please build the DLL first using: cmake --build build --config Release
    pause
    exit /b 1
)

copy /Y "%SOURCE%" "%TARGET%"

if %ERRORLEVEL% EQU 0 (
    echo Successfully copied version.dll
    echo Source: %SOURCE%
    echo Target: %TARGET%
) else (
    echo Failed to copy DLL
    pause
    exit /b 1
)

echo Done!
pause
