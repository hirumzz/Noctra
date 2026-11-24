@echo off
title NOCTRA Desktop Browser Launcher
echo Launching NOCTRA Native Rust Desktop Browser (noctra.exe)...
if exist "%~dp0dist\windows-x64\noctra.exe" (
    start "" "%~dp0dist\windows-x64\noctra.exe"
) else if exist "%~dp0noctra.exe" (
    start "" "%~dp0noctra.exe"
) else if exist "%~dp0target\release\noctra.exe" (
    start "" "%~dp0target\release\noctra.exe"
) else (
    cargo run --release -p noctra-desktop
)
