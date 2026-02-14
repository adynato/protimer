# @powertime/app

Standalone Mac desktop app for tracking Claude Code work time. Built with Tauri and Rust.

## Features

- **Auto-tracking** - Starts/stops when Claude Code is working (via hooks)
- **Manual mode** - Track admin work with play/pause
- **Weekly invoices** - Export billable hours by project
- **Idle detection** - Prompts after 5 minutes of inactivity

## Development

```bash
# From this directory
bun run dev

# Or from repo root
bun run dev --filter=@powertime/app
```

## Build

```bash
bun run build
```

Output: `src-tauri/target/release/bundle/macos/PowerTime.app`
