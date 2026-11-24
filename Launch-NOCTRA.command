#!/usr/bin/env bash
set -e
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$DIR"
if [ -f "$DIR/target/release/noctra-desktop" ]; then
    exec "$DIR/target/release/noctra-desktop"
elif [ -f "$DIR/target/debug/noctra-desktop" ]; then
    exec "$DIR/target/debug/noctra-desktop"
else
    exec cargo run --release -p noctra-desktop
fi
