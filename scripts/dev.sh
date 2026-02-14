#!/bin/bash
# Start UI dev server and Tauri, kill both on Ctrl+C
# Note: API server is started/stopped by Tauri itself

cleanup() {
  echo "Shutting down..."
  kill $UI_PID 2>/dev/null
  exit 0
}

trap cleanup SIGINT SIGTERM

# Start UI dev server
bun run --cwd ui dev &
UI_PID=$!

# Wait for UI to be ready
sleep 1

# Run Tauri (this blocks) - Tauri starts its own API server
cargo tauri dev

cleanup
