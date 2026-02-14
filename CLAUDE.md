# ProTimer

A local-first time tracking tool for freelancers/contractors who use Claude Code. Automatically tracks active work time when Claude is processing prompts.

## Monorepo Structure

```
protimer/
├── packages/
│   └── app/           # Tauri desktop app
│       ├── src-tauri/ # Rust backend
│       └── ui/        # TypeScript frontend
├── turbo.json
└── package.json
```

## Quick Start

```bash
# Install dependencies
bun install

# Run the desktop app
bun run dev --filter=@protimer/app

# Or from packages/app
cd packages/app && bun run dev
```

## Packages

### @protimer/app (Desktop App)

Standalone Mac app built with Tauri and Rust. All data stored locally in SQLite.

**Core Files:**
- `src-tauri/src/lib.rs` - Rust backend with SQLite operations and Tauri commands
- `ui/src/main.ts` - TypeScript frontend using Tauri invoke
- `ui/src/style.css` - Styles

**Data Storage:**
- Database: `~/.protimer/data.db` (SQLite)
- Activity log: `~/.protimer/claude-activity.jsonl`

**Key Features:**
- Multiple project tracking
- Claude Code detection via hooks
- Manual mode (play/pause)
- Invoice generation with PDF output
- Auto hook installation on first launch
- 100% local-first (no cloud, no auth)

## Detection Strategy

Uses **dual-condition validation** for deterministic tracking:

1. **Hook events** - `UserPromptSubmit` = start working, `Stop` = finished responding
2. **Process detection** - Verify a Claude process is running for the project path

A project is only "active" if both conditions are true.

## Claude Code Hooks

On first launch, the app prompts to install hooks automatically. This creates:
- Hook script at `~/.protimer/hooks/track-activity.sh`
- Config in `~/.claude/settings.json`

## Development Notes

- Frontend uses Tauri `invoke()` for all backend communication
- Local `localManualMode` Map provides instant UI response for play/pause
- Rust backend handles all SQLite operations directly
- No HTTP server needed for the app - everything goes through Tauri IPC
