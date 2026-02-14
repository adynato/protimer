# ProTimer

Local-first time tracking for freelancers and contractors who use Claude Code.

<img width="357" height="596" alt="Screenshot 2026-02-14 at 14 49 14" src="https://github.com/user-attachments/assets/64482396-c5ca-45ff-8099-56c9ec0a5706" />


## Background

I built ProTimer while doing contracting work. I was excited by the ability to build new projects with AI and wanted a tool to track my billable hours as I worked with Claude Code.

However, I'm no longer growing my contracting business—I felt too spread thin across multiple projects. Instead of continuing to develop ProTimer commercially, I'm releasing it as open source for anyone else to build on, modify, or commercialize as they see fit.

I'm now focusing on the commitments I already have that are important to me, and leveraging AI depth-first rather than breadth-first. As part of this focus, **I will not be accepting patches or pull requests**, but forks are strongly encouraged.

## Features

- Automatic time tracking when Claude Code is active in your project directory (including nested child folders)
- Manual time tracking with play/pause
- Edit activity time ranges to correct tracking entries
- Multiple project support with hourly rates
- Invoice generation (PDF)
- 100% local - all data stays on your machine
- No cloud, no authentication, no subscription

## Suggested Features

Ideas for forks and extensions:

- Org & team cloud integration
- Screen recording for demos

## Quick Start

```bash
# Install dependencies
bun install
cd packages/app/ui && bun install && cd ../../../

# Run the desktop app
bun run dev
```

## Data Storage

All data is stored locally on your machine:
- Database: `~/.protimer/data.db` (SQLite)
- Activity log: `~/.protimer/claude-activity.jsonl`
- Invoices: `~/.protimer/invoices/`

## License

MIT License - see LICENSE file
