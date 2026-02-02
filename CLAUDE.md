# CC Usage

macOS menu bar app showing Claude Code usage statistics.

## Development

```bash
npm run restart  # Kill app, rebuild, and open (use after making changes)
npm run kill     # Just kill the running app
npm run open     # Open from /Applications symlink
npm run open:dmg # Open the DMG for distribution
npm run link     # Create symlink: /Applications/Claude Usage.app -> build output
npm run dev      # Tauri dev mode with hot reload
```

Always run `npm run restart` after making code changes.

## Setup

1. Build once: `npm run build:app`
2. Create symlink: `npm run link`
3. Add to Login Items: System Settings > General > Login Items > add "Claude Usage"

The symlink means rebuilds automatically update what's in /Applications - no reinstall needed.

## Testing

```bash
cd src-tauri && cargo test
```

## Architecture

- **Tauri 2.x** - Rust backend with web frontend
- **src-tauri/src/lib.rs** - Main Rust code (tray, menu, data fetching, SQLite)
- **dist/index.html** - Web UI with Chart.js for usage graphs
- **~/.claude/cc-usage-cache.json** - Cached usage data
- **~/.claude/cc-usage.db** - SQLite database for historical data

## Data Fetching

Uses tmux automation to run `claude --dangerously-skip-permissions` and capture `/usage` output. Takes ~15 seconds per fetch. Runs every 10 minutes in background.

One nice thing is that this does not require API keys or other credentials.

## Key Files

- `src-tauri/src/lib.rs` - All Rust logic
- `src-tauri/Cargo.toml` - Rust dependencies
- `dist/index.html` - Chart UI (opens on left-click of tray)
- `docs/system-design.md` - Detailed design documentation
