# CC Usage

## Development

```bash
npm run restart  # Kill app, rebuild, and open (use after making changes)
npm run kill     # Just kill the running app
npm run dev      # Tauri dev mode with hot reload
```

Always run `npm run restart` after making code changes.

## Testing

```bash
cd src-tauri && cargo test
```

## Key Files

- `src-tauri/src/lib.rs` - All Rust logic (tray, menu, data fetching, SQLite)
- `src-tauri/Cargo.toml` - Rust dependencies
- `dist/index.html` - Chart UI with Chart.js
- `docs/system-design.md` - Detailed architecture docs
