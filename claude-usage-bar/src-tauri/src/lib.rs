use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Runtime,
};

const CACHE_FILE: &str = ".claude/usage-bar-cache.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct UsageData {
    timestamp: Option<String>,
    session: UsageItem,
    weekly_all: UsageItem,
    weekly_sonnet: UsageItem,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct UsageItem {
    percent: Option<i32>,
    resets: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct AppState {
    usage: UsageData,
    last_error: Option<String>,
    has_network: bool,
    consecutive_errors: u32,
}

fn get_cache_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CACHE_FILE)
}

fn load_cached_usage() -> Option<UsageData> {
    let path = get_cache_path();
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_cached_usage(usage: &UsageData) {
    let path = get_cache_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(usage) {
        let _ = fs::write(path, json);
    }
}

fn get_usage_script() -> String {
    r#"#!/bin/bash
SESSION="claude-usage-$$"
OUTPUT_FILE="/tmp/claude-usage-raw-$$.txt"

cleanup() {
    tmux kill-session -t "$SESSION" 2>/dev/null || true
    rm -f "$OUTPUT_FILE"
}
trap cleanup EXIT

# Network check
if ! ping -c 1 -W 2 api.anthropic.com &>/dev/null; then
    echo '{"error": "No network connection"}'
    exit 0
fi

tmux new-session -d -s "$SESSION" -x 120 -y 50 2>/dev/null
if [ $? -ne 0 ]; then
    echo '{"error": "Failed to start tmux session"}'
    exit 0
fi

tmux send-keys -t "$SESSION" "claude --dangerously-skip-permissions" Enter
sleep 5

tmux send-keys -t "$SESSION" "/usage"
sleep 1
tmux send-keys -t "$SESSION" Enter
sleep 4

tmux capture-pane -t "$SESSION" -p -S -50 > "$OUTPUT_FILE"
tmux send-keys -t "$SESSION" "/exit" Enter
sleep 1

python3 - "$OUTPUT_FILE" << 'PYTHON'
import re, json, sys
from datetime import datetime

try:
    with open(sys.argv[1]) as f:
        content = f.read()
except:
    print('{"error": "Failed to read output"}')
    sys.exit(0)

result = {
    "timestamp": datetime.now().isoformat(),
    "session": {"percent": None, "resets": None},
    "weekly_all": {"percent": None, "resets": None},
    "weekly_sonnet": {"percent": None, "resets": None}
}

current_section = None
for line in content.split("\n"):
    if "Current session" in line:
        current_section = "session"
    elif "Current week (all models)" in line:
        current_section = "weekly_all"
    elif "Current week (Sonnet only)" in line:
        current_section = "weekly_sonnet"

    if pct := re.search(r'(\d+)%\s*used', line):
        if current_section:
            result[current_section]["percent"] = int(pct.group(1))

    if reset := re.search(r'Resets?\s+(.+?)(?:\s*\(|$)', line):
        if current_section:
            result[current_section]["resets"] = reset.group(1).strip()

# Check if we got any data
if result["session"]["percent"] is None and result["weekly_all"]["percent"] is None:
    result["error"] = "Could not parse usage data"

print(json.dumps(result))
PYTHON
"#.to_string()
}

fn fetch_usage() -> UsageData {
    let script = get_usage_script();
    let output = Command::new("bash")
        .arg("-c")
        .arg(&script)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            serde_json::from_str(&stdout).unwrap_or_else(|_| UsageData {
                error: Some("Failed to parse JSON".to_string()),
                ..Default::default()
            })
        }
        Ok(out) => UsageData {
            error: Some(format!("Script failed: {}", String::from_utf8_lossy(&out.stderr))),
            ..Default::default()
        },
        Err(e) => UsageData {
            error: Some(format!("Failed to run script: {}", e)),
            ..Default::default()
        },
    }
}

fn parse_reset_time(resets: &str) -> Option<chrono::DateTime<chrono::Local>> {
    use chrono::{Local, NaiveTime, NaiveDate, TimeZone, Datelike};

    let now = Local::now();

    // Try to parse time like "3pm" or "3:59pm"
    fn parse_time(s: &str) -> Option<NaiveTime> {
        let s = s.trim().to_lowercase();
        let (time_str, is_pm) = if s.ends_with("pm") {
            (s.trim_end_matches("pm"), true)
        } else if s.ends_with("am") {
            (s.trim_end_matches("am"), false)
        } else {
            return None;
        };

        let parts: Vec<&str> = time_str.split(':').collect();
        let hour: u32 = parts.get(0)?.parse().ok()?;
        let minute: u32 = parts.get(1).and_then(|m| m.parse().ok()).unwrap_or(0);

        let hour = if is_pm && hour != 12 { hour + 12 } else if !is_pm && hour == 12 { 0 } else { hour };
        NaiveTime::from_hms_opt(hour, minute, 0)
    }

    if resets.contains(" at ") {
        // e.g., "Jan 29 at 5:59pm"
        let parts: Vec<&str> = resets.split(" at ").collect();
        if parts.len() == 2 {
            let date_str = parts[0].trim();
            let time_str = parts[1].trim();

            // Parse month and day
            let date_parts: Vec<&str> = date_str.split_whitespace().collect();
            if date_parts.len() == 2 {
                let month = match date_parts[0].to_lowercase().as_str() {
                    "jan" => 1, "feb" => 2, "mar" => 3, "apr" => 4,
                    "may" => 5, "jun" => 6, "jul" => 7, "aug" => 8,
                    "sep" => 9, "oct" => 10, "nov" => 11, "dec" => 12,
                    _ => return None,
                };
                let day: u32 = date_parts[1].parse().ok()?;
                let year = if month < now.month() || (month == now.month() && day < now.day()) {
                    now.year() + 1
                } else {
                    now.year()
                };

                let time = parse_time(time_str)?;
                let date = NaiveDate::from_ymd_opt(year, month, day)?;
                let datetime = date.and_time(time);
                return Local.from_local_datetime(&datetime).single();
            }
        }
    } else {
        // Just time like "3pm" - assume today
        let time = parse_time(resets)?;
        let datetime = now.date_naive().and_time(time);
        let result = Local.from_local_datetime(&datetime).single()?;
        // If time has passed, assume tomorrow
        if result < now {
            let tomorrow = now.date_naive().succ_opt()?;
            let datetime = tomorrow.and_time(time);
            return Local.from_local_datetime(&datetime).single();
        }
        return Some(result);
    }

    None
}

fn format_duration(duration: chrono::Duration) -> String {
    let total_hours = duration.num_hours();
    let days = total_hours / 24;
    let hours = total_hours % 24;

    if days > 0 {
        format!("{}d {}h left", days, hours)
    } else if hours > 0 {
        format!("{}h left", hours)
    } else {
        let mins = duration.num_minutes();
        if mins > 0 {
            format!("{}m left", mins)
        } else {
            "soon".to_string()
        }
    }
}

fn format_time_remaining(resets: &str) -> String {
    let now = chrono::Local::now();

    if let Some(reset_time) = parse_reset_time(resets) {
        let duration = reset_time.signed_duration_since(now);
        if duration.num_seconds() > 0 {
            return format_duration(duration);
        }
    }

    // Fallback to showing the raw reset time
    if resets.contains("at") {
        format!("Resets {}", resets)
    } else {
        format!("Resets today {}", resets)
    }
}

// Calculate time-based status indicator
// If usage % is higher than time elapsed %, you're over-pace
fn get_status_indicator_simple(percent: i32) -> &'static str {
    if percent >= 90 {
        "🔴"
    } else if percent >= 75 {
        "🟠"
    } else if percent >= 50 {
        "🟡"
    } else {
        "🟢"
    }
}

// Get status based on usage vs time elapsed
// period_hours: total period length (4 for session, 168 for week)
fn get_status_indicator_paced(usage_percent: i32, resets: Option<&str>, period_hours: i32) -> &'static str {
    // Calculate how much time has elapsed as a percentage
    let time_percent = if let Some(reset_str) = resets {
        if let Some(reset_time) = parse_reset_time(reset_str) {
            let now = chrono::Local::now();
            let remaining = reset_time.signed_duration_since(now);
            let remaining_hours = remaining.num_hours() as i32;
            let elapsed_hours = period_hours - remaining_hours;
            if period_hours > 0 {
                ((elapsed_hours as f32 / period_hours as f32) * 100.0) as i32
            } else {
                50 // fallback
            }
        } else {
            50 // can't parse, assume midpoint
        }
    } else {
        50 // no reset info, assume midpoint
    };

    // Compare usage to time elapsed
    // If usage is 20%+ ahead of time, red
    // If usage is 10%+ ahead of time, orange
    // If usage is ahead but <10%, yellow
    // Otherwise green
    let pace_diff = usage_percent - time_percent;

    if usage_percent >= 90 {
        "🔴" // Always red at 90%+
    } else if pace_diff >= 20 {
        "🔴"
    } else if pace_diff >= 10 {
        "🟠"
    } else if pace_diff > 0 {
        "🟡"
    } else {
        "🟢"
    }
}

fn build_menu<R: Runtime>(app: &tauri::AppHandle<R>, state: &AppState) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(app)?;
    let usage = &state.usage;

    // Show error if present
    if let Some(ref err) = state.last_error {
        let err_text = format!("⚠️ {}", err);
        menu.append(&MenuItem::new(app, &err_text, false, None::<&str>)?)?;
        menu.append(&MenuItem::new(app, "─────────────", false, None::<&str>)?)?;
    }

    // Session info (4 hour period for Opus)
    let session_pct = usage.session.percent.unwrap_or(0);
    let session_reset = usage.session.resets.as_deref();
    let session_indicator = get_status_indicator_paced(session_pct, session_reset, 4);
    let session_reset_display = session_reset.unwrap_or("--");
    let session_text = format!(
        "{} Session: {}% | {}",
        session_indicator, session_pct, format_time_remaining(session_reset_display)
    );
    menu.append(&MenuItem::new(app, &session_text, false, None::<&str>)?)?;

    // Weekly all models (7 day = 168 hour period)
    let weekly_pct = usage.weekly_all.percent.unwrap_or(0);
    let weekly_reset = usage.weekly_all.resets.as_deref();
    let weekly_indicator = get_status_indicator_paced(weekly_pct, weekly_reset, 168);
    let weekly_reset_display = weekly_reset.unwrap_or("--");
    let weekly_text = format!(
        "{} Weekly (all): {}% | {}",
        weekly_indicator, weekly_pct, format_time_remaining(weekly_reset_display)
    );
    menu.append(&MenuItem::new(app, &weekly_text, false, None::<&str>)?)?;

    // Weekly Sonnet (also 7 day period)
    if let Some(sonnet_pct) = usage.weekly_sonnet.percent {
        let sonnet_reset = usage.weekly_sonnet.resets.as_deref();
        let sonnet_indicator = get_status_indicator_paced(sonnet_pct, sonnet_reset, 168);
        let sonnet_text = format!("{} Weekly (Sonnet): {}%", sonnet_indicator, sonnet_pct);
        menu.append(&MenuItem::new(app, &sonnet_text, false, None::<&str>)?)?;
    }

    // Timestamp - show absolute time (HH:mm:ss if today, otherwise date + time)
    if let Some(ref ts) = usage.timestamp {
        let display = {
            let ts_clean = ts.split('.').next().unwrap_or(ts);
            if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(ts_clean, "%Y-%m-%dT%H:%M:%S") {
                let now = chrono::Local::now().naive_local();
                let today = now.date();
                let parsed_date = parsed.date();

                if today == parsed_date {
                    // Same day - just show time
                    parsed.format("%H:%M:%S").to_string()
                } else {
                    // Different day - show date and time
                    parsed.format("%b %d %H:%M:%S").to_string()
                }
            } else {
                ts.clone()
            }
        };
        menu.append(&MenuItem::new(app, &format!("Updated: {}", display), false, None::<&str>)?)?;
    }

    // Separator and actions
    menu.append(&MenuItem::new(app, "─────────────", false, None::<&str>)?)?;

    let refresh = MenuItem::with_id(app, "refresh", "Refresh Now", true, None::<&str>)?;
    menu.append(&refresh)?;

    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    menu.append(&quit)?;

    Ok(menu)
}

fn get_tray_title(state: &AppState) -> String {
    if state.last_error.is_some() {
        "⚠️".to_string()
    } else if state.usage.session.percent.is_some() {
        format!(
            "{}% {}%",
            state.usage.session.percent.unwrap_or(0),
            state.usage.weekly_all.percent.unwrap_or(0)
        )
    } else {
        "...".to_string()
    }
}

// Load the orange asterisk tray icon
fn load_tray_icon() -> Image<'static> {
    let icon_bytes = include_bytes!("../icons/tray-icon.png");
    Image::from_bytes(icon_bytes).expect("Failed to load tray icon")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Load cached data on startup
    let initial_usage = load_cached_usage().unwrap_or_default();

    let app_state: Arc<Mutex<AppState>> = Arc::new(Mutex::new(AppState {
        usage: initial_usage,
        has_network: true,
        ..Default::default()
    }));

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {
            // Another instance tried to start - we could focus window here if we had one
            // For tray-only app, just ignore
        }))
        .setup(move |app| {
            let handle = app.handle().clone();
            let state_for_tray = app_state.clone();
            let state_for_menu = app_state.clone();

            // Build initial menu with cached data
            let initial_state = state_for_tray.lock().unwrap();
            let initial_menu = build_menu(&handle, &initial_state)?;
            let initial_title = get_tray_title(&initial_state);
            drop(initial_state);

            // Create tray with ID - only one!
            let tray_icon = load_tray_icon();
            let _tray = TrayIconBuilder::with_id("main")
                .icon(tray_icon)
                .menu(&initial_menu)
                .tooltip("Claude Code Usage")
                .title(&initial_title)
                .on_menu_event(move |app, event| {
                    match event.id.as_ref() {
                        "quit" => {
                            app.exit(0);
                        }
                        "refresh" => {
                            let data = fetch_usage();
                            let mut state = state_for_menu.lock().unwrap();
                            if let Some(ref err) = data.error {
                                state.last_error = Some(err.clone());
                            } else {
                                save_cached_usage(&data);
                                state.usage = data;
                                state.last_error = None;
                                state.consecutive_errors = 0;
                            }
                            // Update menu
                            if let Some(tray) = app.tray_by_id("main") {
                                let _ = tray.set_title(Some(&get_tray_title(&state)));
                                if let Ok(menu) = build_menu(app, &state) {
                                    let _ = tray.set_menu(Some(menu));
                                }
                            }
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            // Spawn background data fetch task (every 10 min)
            let handle_for_refresh = app.handle().clone();
            let state_for_refresh = app_state.clone();

            std::thread::spawn(move || {
                let mut first_run = true;

                loop {
                    if !first_run {
                        let state = state_for_refresh.lock().unwrap();
                        let sleep_secs = if state.consecutive_errors > 0 {
                            600 * std::cmp::min(state.consecutive_errors, 3)
                        } else {
                            600 // 10 minutes
                        };
                        drop(state);
                        std::thread::sleep(Duration::from_secs(sleep_secs.into()));
                    }
                    first_run = false;

                    let data = fetch_usage();
                    let mut state = state_for_refresh.lock().unwrap();

                    if let Some(ref err) = data.error {
                        state.last_error = Some(err.clone());
                        state.consecutive_errors += 1;
                        state.has_network = !err.contains("No network");
                    } else {
                        save_cached_usage(&data);
                        state.usage = data;
                        state.last_error = None;
                        state.consecutive_errors = 0;
                        state.has_network = true;
                    }

                    let title = get_tray_title(&state);
                    let state_clone = state.clone();
                    drop(state);

                    if let Some(tray) = handle_for_refresh.tray_by_id("main") {
                        let _ = tray.set_title(Some(&title));
                        if let Ok(menu) = build_menu(&handle_for_refresh, &state_clone) {
                            let _ = tray.set_menu(Some(menu));
                        }
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relative_time_parsing() {
        // Test timestamp from 5 minutes ago
        let now = chrono::Local::now().naive_local();
        let five_mins_ago = now - chrono::Duration::minutes(5);
        let ts = five_mins_ago.format("%Y-%m-%dT%H:%M:%S%.f").to_string();

        let ts_clean = ts.split('.').next().unwrap_or(&ts);
        let parsed = chrono::NaiveDateTime::parse_from_str(ts_clean, "%Y-%m-%dT%H:%M:%S").unwrap();
        let duration = now.signed_duration_since(parsed);
        let mins = duration.num_minutes();

        assert!(mins >= 4 && mins <= 6, "Expected ~5 mins, got {}", mins);
    }

    #[test]
    fn test_relative_time_seconds() {
        let now = chrono::Local::now().naive_local();
        let ts = now.format("%Y-%m-%dT%H:%M:%S%.f").to_string();

        let ts_clean = ts.split('.').next().unwrap_or(&ts);
        let parsed = chrono::NaiveDateTime::parse_from_str(ts_clean, "%Y-%m-%dT%H:%M:%S").unwrap();
        let duration = now.signed_duration_since(parsed);
        let secs = duration.num_seconds();

        assert!(secs < 2, "Expected <2 secs, got {}", secs);
    }

    #[test]
    fn test_parse_reset_time_today() {
        let result = parse_reset_time("3pm");
        assert!(result.is_some(), "Should parse '3pm'");
    }

    #[test]
    fn test_parse_reset_time_future_date() {
        let result = parse_reset_time("Jan 29 at 5:59pm");
        assert!(result.is_some(), "Should parse 'Jan 29 at 5:59pm'");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(chrono::Duration::hours(25)), "1d 1h left");
        assert_eq!(format_duration(chrono::Duration::hours(5)), "5h left");
        assert_eq!(format_duration(chrono::Duration::minutes(30)), "30m left");
    }

    #[test]
    fn test_timestamp_display_same_day() {
        let now = chrono::Local::now().naive_local();
        let ts = now.format("%Y-%m-%dT%H:%M:%S").to_string();

        let ts_clean = ts.split('.').next().unwrap_or(&ts);
        let parsed = chrono::NaiveDateTime::parse_from_str(ts_clean, "%Y-%m-%dT%H:%M:%S").unwrap();
        let today = now.date();
        let parsed_date = parsed.date();

        assert_eq!(today, parsed_date, "Should be same day");

        let display = parsed.format("%H:%M:%S").to_string();
        assert!(display.contains(":"), "Should be HH:MM:SS format: {}", display);
        assert!(!display.contains("Jan") && !display.contains("Feb"), "Should not contain month");
    }

    #[test]
    fn test_timestamp_display_different_day() {
        let now = chrono::Local::now().naive_local();
        let yesterday = now - chrono::Duration::days(1);
        let ts = yesterday.format("%Y-%m-%dT%H:%M:%S").to_string();

        let ts_clean = ts.split('.').next().unwrap_or(&ts);
        let parsed = chrono::NaiveDateTime::parse_from_str(ts_clean, "%Y-%m-%dT%H:%M:%S").unwrap();
        let today = now.date();
        let parsed_date = parsed.date();

        assert_ne!(today, parsed_date, "Should be different day");

        let display = parsed.format("%b %d %H:%M:%S").to_string();
        assert!(display.contains(" "), "Should contain date: {}", display);
    }

    #[test]
    fn test_pace_indicator_under_pace() {
        // 30% usage with 50% time elapsed = under pace = green
        let indicator = get_status_indicator_paced(30, Some("3pm"), 4);
        assert_eq!(indicator, "🟢", "Under pace should be green");
    }

    #[test]
    fn test_pace_indicator_over_pace() {
        // 90% usage = always red regardless of pace
        let indicator = get_status_indicator_paced(90, Some("3pm"), 4);
        assert_eq!(indicator, "🔴", "90%+ should always be red");
    }

    #[test]
    fn test_parse_time_am_pm() {
        // Test various time formats
        assert!(parse_reset_time("3pm").is_some());
        assert!(parse_reset_time("12am").is_some());
        assert!(parse_reset_time("11:59pm").is_some());
        assert!(parse_reset_time("1:30am").is_some());
    }

    #[test]
    fn test_parse_date_time() {
        assert!(parse_reset_time("Jan 29 at 5:59pm").is_some());
        assert!(parse_reset_time("Feb 1 at 12am").is_some());
        assert!(parse_reset_time("Dec 31 at 11:59pm").is_some());
    }

    #[test]
    fn test_format_duration_edge_cases() {
        assert_eq!(format_duration(chrono::Duration::seconds(30)), "soon"); // <1 min
        assert_eq!(format_duration(chrono::Duration::minutes(1)), "1m left");
        assert_eq!(format_duration(chrono::Duration::hours(0)), "soon");
        assert_eq!(format_duration(chrono::Duration::hours(48)), "2d 0h left");
        assert_eq!(format_duration(chrono::Duration::hours(49)), "2d 1h left");
    }
}
