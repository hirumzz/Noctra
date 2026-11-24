@echo off
title NOCTRA Desktop Browser Launcher
echo Launching NOCTRA Native Rust Desktop Browser...
if exist "%~dp0target\release\noctra-desktop.exe" (
    start "" "%~dp0target\release\noctra-desktop.exe"
) else (
    cargo run --release -p noctra-desktop
)
