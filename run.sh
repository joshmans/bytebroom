#!/bin/sh
# Builds if needed, then opens the app. Extra arguments are passed through (e.g. ./run.sh --print).
cd "$(dirname "$0")" && exec cargo run --release -- "$@"
