@echo off
title NOCTRA Desktop Browser — Unblock & Launch
echo Unblocking Windows SmartScreen Mark-of-the-Web (Zone.Identifier)...
powershell -NoProfile -ExecutionPolicy Bypass -Command "Unblock-File -Path '%~dp0dist\windows-x64\noctra.exe' -ErrorAction SilentlyContinue; Unblock-File -Path '%~dp0noctra.exe' -ErrorAction SilentlyContinue"

echo Launching NOCTRA Desktop Browser (noctra.exe)...
if exist "%~dp0dist\windows-x64\noctra.exe" (
    start "" "%~dp0dist\windows-x64\noctra.exe"
) else if exist "%~dp0noctra.exe" (
    start "" "%~dp0noctra.exe"
) else if exist "%~dp0target\release\noctra.exe" (
    start "" "%~dp0target\release\noctra.exe"
) else (
    cargo run --release -p noctra-desktop
)
