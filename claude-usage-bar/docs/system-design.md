# Claude Usage Bar - System Design

A macOS menu bar app that displays Claude Code usage statistics.

## Overview

This app shows real-time Claude Code usage in the macOS menu bar, including:
- Session usage percentage (4-hour Opus window)
- Weekly usage percentage (7-day window)
- Time remaining until reset
- Color-coded status indicators based on usage pace

## Architecture

### Tech Stack

**Tauri 2.x** was chosen over alternatives:
- **vs Electron**: Much smaller binary (~12MB vs ~150MB+), lower memory, native performance
- **vs Swift/AppKit native**: Faster iteration with web tech for UI, cross-platform potential
- **vs xbar/SwiftBar**: More control, custom features, no dependency on third-party menu bar tools

**Components:**
- `src-tauri/src/lib.rs` - Rust backend (tray, menu, data fetching)
- `src-tauri/icons/tray-icon.png` - Claude symbol icon
- `~/.claude/usage-bar-cache.json` - Persisted usage data

### Data Fetching Strategy

**Problem**: Claude Code's `/usage` command only works in interactive mode. There's no CLI flag or API endpoint for usage data.

**Solution**: tmux automation
1. Start a detached tmux session
2. Launch `claude --dangerously-skip-permissions` (bypasses trust prompt)
3. Send `/usage` command
4. Capture pane output
5. Parse with Python regex
6. Return JSON

**Why this approach:**
- No authentication tokens needed (uses existing Claude CLI auth)
- No unofficial API scraping
- Works with any Claude subscription type
- Data stays local

**Rejected alternatives:**
- **Session key from browser**: Requires user to copy cookies, security concerns
- **CodexBar/Usage4Claude**: Asked for confidential info, user uncomfortable
- **Admin API**: Only for organizations, not individual accounts
- **Local stats-cache.json only**: Doesn't have real-time quota/reset info

### Refresh Strategy

**Two-tier refresh:**

1. **Data fetch** (every 10 minutes):
   - Runs tmux/claude automation
   - Takes ~15 seconds due to Claude startup time
   - Updates cached data and menu

2. **Display refresh** (every 30 seconds):
   - Just rebuilds menu from cached state
   - Updates relative timestamps ("2m30s ago" → "3m ago")
   - No API/CLI calls

**Why not refresh on tray click:**
Native macOS menus can't be updated while open. Attempting to rebuild on click causes the menu to flash and close.

### Color-Coded Status Indicators

**Pace-based coloring** (not just absolute percentage):

```
Usage% vs Time Elapsed% → Color
─────────────────────────────────
20%+ ahead of pace     → 🔴 Red
10-20% ahead of pace   → 🟠 Orange
0-10% ahead of pace    → 🟡 Yellow
On pace or under       → 🟢 Green
Always at 90%+ usage   → 🔴 Red
```

**Time periods:**
- Session: 4 hours (Opus)
- Weekly: 168 hours (7 days)

**Example**: If 3 days (43%) have passed and you've used 60% of weekly quota, you're 17% ahead of pace → Orange warning.

### Timestamp Display

**Precise relative time** instead of vague "just now":
- `15s ago`
- `2m30s ago`
- `1h15m ago`
- `2d5h ago`

### Icon

Uses the official Claude AI symbol from Wikimedia Commons, converted to 22x22 PNG with transparent background for macOS menu bar.

## Data Flow

```
┌─────────────────────────────────────────────────────────┐
│                     On App Start                         │
├─────────────────────────────────────────────────────────┤
│ 1. Load ~/.claude/usage-bar-cache.json (if exists)      │
│ 2. Display cached data immediately                       │
│ 3. Start background fetch (gets fresh data)             │
│ 4. Update display when fetch completes                  │
└─────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────┐
│                  Every 10 Minutes                        │
├─────────────────────────────────────────────────────────┤
│ 1. Check network (ping api.anthropic.com)               │
│ 2. If no network: show last cached data, retry later    │
│ 3. Run tmux automation to get /usage output             │
│ 4. Parse percentages and reset times                    │
│ 5. Save to cache file                                   │
│ 6. Update tray title and menu                           │
└─────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────┐
│                  Every 30 Seconds                        │
├─────────────────────────────────────────────────────────┤
│ 1. Rebuild menu from current state (no fetch)           │
│ 2. Timestamps update: "2m ago" → "2m30s ago"            │
└─────────────────────────────────────────────────────────┘
```

## Error Handling

**Network errors:**
- Show warning icon in menu bar: `⚠️`
- Display last known good data
- Exponential backoff: 10min → 20min → 30min max

**Parse errors:**
- Log error in menu dropdown
- Keep previous valid data
- Retry on next cycle

## Evolution & Design Decisions

### Initial Exploration

1. **Evaluated existing apps** (CodexBar, Usage4Claude)
   - CodexBar: 1.9k stars, multi-provider support
   - Usage4Claude: 122 stars, Claude-focused
   - Both required session keys from browser cookies
   - User uncomfortable with credential extraction

2. **Investigated official APIs**
   - Admin API exists but requires organization account
   - No public usage endpoint for individual users
   - `/usage` CLI command works but only in interactive mode

### Key Pivots

1. **tmux automation** emerged as the solution
   - Discovered `--dangerously-skip-permissions` flag bypasses trust prompt
   - Enables fully automated, non-interactive usage fetching

2. **Switched from transparent icon to Claude symbol**
   - Initially tried transparent 1x1 PNG (icon still showed)
   - User requested actual Claude symbol
   - Downloaded from Wikimedia, converted to 22x22 PNG

3. **Timestamp precision increased**
   - Started with "just now" / "5m ago"
   - User wanted precise: "2m30s ago"
   - Added second-level granularity

4. **Menu refresh strategy evolved**
   - Tried refreshing on tray click → caused flash/close
   - Settled on 30-second background refresh for timestamps
   - Separate 10-minute cycle for actual data fetch

### Rejected Features

- **Notifications at threshold**: Not implemented yet, could add later
- **Historical charts**: stats-cache.json has data, but adds complexity
- **Multiple account support**: Out of scope for v1

## File Structure

```
claude-usage-bar/
├── docs/
│   └── system-design.md          # This file
├── dist/
│   └── index.html                # Minimal (tray-only app)
├── src-tauri/
│   ├── icons/
│   │   ├── tray-icon.png         # Claude symbol 22x22
│   │   └── tray-icon@2x.png      # Retina version
│   ├── src/
│   │   └── lib.rs                # All Rust code
│   ├── Cargo.toml
│   └── tauri.conf.json
├── package.json
└── tmp/                          # Gitignored temp files
```

## Building

```bash
npm install
npm run build:app
```

Output: `src-tauri/target/release/bundle/macos/Claude Usage.app`

## Testing

```bash
cd src-tauri
cargo test
```

Tests cover:
- Timestamp parsing and relative time calculation
- Reset time parsing ("3pm", "Jan 29 at 5:59pm")
- Duration formatting

## Future Considerations

1. **Notifications**: Alert at configurable usage thresholds (50%, 75%, 90%)
2. **Launch at login**: Add to Login Items automatically
3. **Feature request upstream**: `claude usage` CLI subcommand would eliminate tmux hack
4. **Preferences window**: Configure refresh interval, thresholds, etc.
