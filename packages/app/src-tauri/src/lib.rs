use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tauri::{State, Emitter};
use std::os::unix::fs::PermissionsExt;
use notify::{Watcher, RecursiveMode, Event, EventKind};
use std::sync::mpsc::channel;

mod invoice;

// Cache for activity log and system idle time
struct ActivityCache {
    entries: Arc<Vec<ActivityEntry>>,
    file_modified: Option<SystemTime>,
    system_idle_time: i64,
    system_idle_checked: i64,
}

// Database connection wrapped in Mutex for thread safety
struct AppState {
    db: Mutex<Connection>,
    cache: Mutex<ActivityCache>,
}

// Data types matching the TypeScript interfaces
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub color: String,
    pub hourly_rate: Option<f64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BusinessInfo {
    pub name: String,
    pub email: Option<String>,
    pub tax_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeEntry {
    pub id: String,
    pub project_id: String,
    pub start_time: i64,
    pub end_time: Option<i64>,
    pub claude_code_active: bool,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSession {
    pub project_id: String,
    pub start_time: i64,
    pub claude_code_detected: bool,
    pub last_claude_check: i64,
    pub manual_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStatus {
    #[serde(flatten)]
    pub project: Project,
    pub is_tracking: bool,
    pub manual_mode: bool,
    pub elapsed_time: i64,
    pub today_time: i64,
    pub week_time: i64,
    pub total_time: i64,
    pub claude_state: String,
    pub claude_session_count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub projects: Vec<ProjectStatus>,
    pub today_total: i64,
    pub claude_total: i64,
    pub system_idle_time: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeeklySummaryProject {
    pub project_id: String,
    pub project_name: String,
    pub total_ms: i64,
    pub total_hours: f64,
    pub entry_count: i32,
    pub hourly_rate: Option<f64>,
    pub earnings: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeeklySummary {
    pub week_start: String,
    pub week_end: String,
    pub projects: Vec<WeeklySummaryProject>,
    pub total_earnings: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceRecord {
    pub invoice_number: String,
    pub project_id: String,
    pub project_name: String,
    pub file_path: String,
    pub start_date: i64,
    pub end_date: i64,
    pub total_amount: f64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct ActivityEntry {
    event: String,
    session_id: String,
    cwd: Option<String>,
    timestamp: i64,
}

// Get the data directory path
fn get_data_dir() -> PathBuf {
    let home = dirs::home_dir().expect("Could not find home directory");
    home.join(".protimer")
}

fn get_db_path() -> PathBuf {
    get_data_dir().join("data.db")
}

fn get_activity_log_path() -> PathBuf {
    get_data_dir().join("claude-activity.jsonl")
}

// Initialize database
fn init_db(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            path TEXT NOT NULL UNIQUE,
            color TEXT NOT NULL,
            createdAt INTEGER NOT NULL
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS time_entries (
            id TEXT PRIMARY KEY,
            projectId TEXT NOT NULL,
            startTime INTEGER NOT NULL,
            endTime INTEGER,
            claudeCodeActive INTEGER NOT NULL DEFAULT 0,
            description TEXT,
            FOREIGN KEY (projectId) REFERENCES projects(id)
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS active_sessions (
            projectId TEXT PRIMARY KEY,
            startTime INTEGER NOT NULL,
            claudeCodeDetected INTEGER NOT NULL DEFAULT 0,
            lastClaudeCheck INTEGER NOT NULL,
            manualMode INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (projectId) REFERENCES projects(id)
        )",
        [],
    )?;

    // Migration: add manualMode column if it doesn't exist
    let _ = conn.execute(
        "ALTER TABLE active_sessions ADD COLUMN manualMode INTEGER NOT NULL DEFAULT 0",
        [],
    );

    // Migration: add hourlyRate column to projects
    let _ = conn.execute(
        "ALTER TABLE projects ADD COLUMN hourlyRate REAL",
        [],
    );

    // Create business_info table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS business_info (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            name TEXT NOT NULL DEFAULT '',
            address TEXT NOT NULL DEFAULT '',
            email TEXT NOT NULL DEFAULT '',
            phone TEXT NOT NULL DEFAULT '',
            taxRate REAL NOT NULL DEFAULT 0.0,
            invoiceCounter INTEGER NOT NULL DEFAULT 1
        )",
        [],
    )?;

    // Insert default business info if not exists
    let _ = conn.execute(
        "INSERT OR IGNORE INTO business_info (id, name, address, email, phone, taxRate, invoiceCounter)
         VALUES (1, '', '', '', '', 0.0, 1)",
        [],
    );

    // Create invoices table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS invoices (
            id TEXT PRIMARY KEY,
            invoiceNumber TEXT NOT NULL,
            projectId TEXT NOT NULL,
            filePath TEXT NOT NULL,
            startDate INTEGER NOT NULL,
            endDate INTEGER NOT NULL,
            totalAmount REAL NOT NULL,
            createdAt INTEGER NOT NULL,
            FOREIGN KEY (projectId) REFERENCES projects(id)
        )",
        [],
    )?;

    // Migration: add client fields to projects
    let _ = conn.execute(
        "ALTER TABLE projects ADD COLUMN clientName TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE projects ADD COLUMN clientEmail TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE projects ADD COLUMN clientAddress TEXT",
        [],
    );

    // Performance indexes
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_time_entries_project_start ON time_entries(projectId, startTime)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_time_entries_claude ON time_entries(claudeCodeActive)",
        [],
    );

    Ok(())
}

// Generate unique ID
fn generate_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

// Get current timestamp in milliseconds
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

// Check if cwd_path is within project_path (same or subfolder only)
fn is_path_within_project(cwd_path: &str, project_path: &str) -> bool {
    let cwd = cwd_path.trim_end_matches('/');
    let project = project_path.trim_end_matches('/');

    // Exact match
    if cwd == project {
        return true;
    }
    // cwd is a subfolder of project
    if cwd.starts_with(&format!("{}/", project)) {
        return true;
    }
    // Don't match parent directories - too aggressive
    false
}

// Refresh activity log cache if file changed
fn refresh_activity_cache(cache: &mut ActivityCache) {
    let log_path = get_activity_log_path();

    let current_modified = fs::metadata(&log_path)
        .ok()
        .and_then(|m| m.modified().ok());

    let needs_refresh = match (&cache.file_modified, &current_modified) {
        (Some(cached), Some(current)) => cached != current,
        (None, Some(_)) => true,
        _ => false,
    };

    if needs_refresh {
        let mut new_entries = Vec::new();
        if let Ok(file) = fs::File::open(&log_path) {
            let reader = BufReader::new(file);
            for line in reader.lines().map_while(Result::ok) {
                if let Ok(entry) = serde_json::from_str::<ActivityEntry>(&line) {
                    new_entries.push(entry);
                }
            }
        }
        cache.entries = Arc::new(new_entries);
        cache.file_modified = current_modified;
    }
}


// Get Claude sessions for a project from cached activity log
// Hooks are source of truth for starting, process detection is fallback for stopping
fn get_claude_sessions_for_project_cached(
    project_path: &str,
    entries: &[ActivityEntry],
) -> Vec<(String, String, i64)> {
    let now = now_ms();
    // Sessions older than 10 minutes with no Stop are considered stale
    let stale_threshold = 10 * 60 * 1000; // 10 minutes in ms

    let mut sessions: std::collections::HashMap<String, (String, i64)> = std::collections::HashMap::new();

    for entry in entries {
        if let Some(cwd) = &entry.cwd {
            if is_path_within_project(cwd, project_path) {
                let state = if entry.event == "UserPromptSubmit" {
                    "active"
                } else {
                    "stopped"
                };
                sessions.insert(entry.session_id.clone(), (state.to_string(), entry.timestamp));
            }
        }
    }

    // Filter out stale "active" sessions - if last activity was > 10 min ago, treat as stopped
    sessions
        .into_iter()
        .map(|(id, (state, ts))| {
            if state == "active" && (now - ts) > stale_threshold {
                (id, "stopped".to_string(), ts)
            } else {
                (id, state, ts)
            }
        })
        .collect()
}

// Get system idle time (macOS) - actual implementation
fn do_get_system_idle_time() -> i64 {
    if let Ok(output) = Command::new("ioreg")
        .args(["-c", "IOHIDSystem"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("HIDIdleTime") {
                if let Some(val) = line.split('=').nth(1) {
                    if let Ok(ns) = val.trim().parse::<i64>() {
                        return ns / 1_000_000; // Convert ns to ms
                    }
                }
            }
        }
    }
    0
}

// Refresh system idle time cache (every 5 seconds)
fn refresh_system_idle_cache(cache: &mut ActivityCache) {
    let now = now_ms();
    if now - cache.system_idle_checked > 5000 {
        cache.system_idle_time = do_get_system_idle_time();
        cache.system_idle_checked = now;
    }
}

// Get start of today in milliseconds
fn get_today_start_ms() -> i64 {
    let now = chrono::Local::now();
    let today = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
    today.and_local_timezone(chrono::Local).unwrap().timestamp_millis()
}

// Get start of week (Monday) in milliseconds
fn get_week_start_ms() -> i64 {
    use chrono::{Datelike, Duration, Local};
    let now = Local::now();
    let days_since_monday = now.weekday().num_days_from_monday() as i64;
    let monday = now.date_naive() - Duration::days(days_since_monday);
    monday.and_hms_opt(0, 0, 0).unwrap()
        .and_local_timezone(Local).unwrap()
        .timestamp_millis()
}

// ============== HOOK MANAGEMENT ==============

fn get_hooks_dir() -> PathBuf {
    get_data_dir().join("hooks")
}

fn get_hook_script_path() -> PathBuf {
    get_hooks_dir().join("track-activity.sh")
}

fn get_claude_settings_path() -> PathBuf {
    dirs::home_dir()
        .expect("Could not find home directory")
        .join(".claude")
        .join("settings.json")
}

const HOOK_SCRIPT: &str = r#"#!/bin/bash
# Claude Code Activity Hook for ProTimer
# This script is called by Claude Code hooks to track when Claude is actively working

# Activity log location - shared across all projects
ACTIVITY_DIR="$HOME/.protimer"
ACTIVITY_LOG="$ACTIVITY_DIR/claude-activity.jsonl"

# Ensure directory exists
mkdir -p "$ACTIVITY_DIR"

# Read hook input from stdin
input=$(cat)

# Parse event details
event=$(echo "$input" | jq -r '.hook_event_name // "unknown"')
session_id=$(echo "$input" | jq -r '.session_id // "unknown"')
tool_name=$(echo "$input" | jq -r '.tool_name // "none"')
cwd=$(echo "$input" | jq -r '.cwd // "unknown"')
timestamp=$(($(date +%s) * 1000))  # Unix timestamp in milliseconds (macOS compatible)

# Log the activity
echo "{\"event\":\"$event\",\"session_id\":\"$session_id\",\"tool\":\"$tool_name\",\"cwd\":\"$cwd\",\"timestamp\":$timestamp}" >> "$ACTIVITY_LOG"

# Keep log file from growing too large (keep last 1000 lines)
if [ $(wc -l < "$ACTIVITY_LOG") -gt 1000 ]; then
  tail -500 "$ACTIVITY_LOG" > "$ACTIVITY_LOG.tmp" && mv "$ACTIVITY_LOG.tmp" "$ACTIVITY_LOG"
fi

exit 0
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HooksStatus {
    pub script_installed: bool,
    pub settings_configured: bool,
    pub fully_installed: bool,
}

fn check_hooks_status() -> HooksStatus {
    let script_path = get_hook_script_path();
    let settings_path = get_claude_settings_path();

    let script_installed = script_path.exists();

    let settings_configured = if let Ok(content) = fs::read_to_string(&settings_path) {
        // Check if settings contain our hook path
        let hook_path = script_path.to_string_lossy();
        content.contains(&*hook_path) || content.contains("/.protimer/hooks/track-activity.sh")
    } else {
        false
    };

    HooksStatus {
        script_installed,
        settings_configured,
        fully_installed: script_installed && settings_configured,
    }
}

fn do_install_hooks() -> Result<(), String> {
    let hooks_dir = get_hooks_dir();
    let script_path = get_hook_script_path();
    let settings_path = get_claude_settings_path();

    // Create hooks directory
    fs::create_dir_all(&hooks_dir).map_err(|e| format!("Failed to create hooks directory: {}", e))?;

    // Write hook script
    let mut file = fs::File::create(&script_path)
        .map_err(|e| format!("Failed to create hook script: {}", e))?;
    file.write_all(HOOK_SCRIPT.as_bytes())
        .map_err(|e| format!("Failed to write hook script: {}", e))?;

    // Make executable (chmod +x)
    let mut perms = fs::metadata(&script_path)
        .map_err(|e| format!("Failed to get script metadata: {}", e))?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script_path, perms)
        .map_err(|e| format!("Failed to set script permissions: {}", e))?;

    // Update Claude settings
    let claude_dir = settings_path.parent().unwrap();
    fs::create_dir_all(claude_dir).map_err(|e| format!("Failed to create .claude directory: {}", e))?;

    let hook_command = script_path.to_string_lossy().to_string();

    // Read existing settings or create new
    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = fs::read_to_string(&settings_path)
            .map_err(|e| format!("Failed to read Claude settings: {}", e))?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Ensure hooks object exists
    if settings.get("hooks").is_none() {
        settings["hooks"] = serde_json::json!({});
    }

    let hooks = settings.get_mut("hooks").unwrap();

    // Add UserPromptSubmit hook
    let user_prompt_hook = serde_json::json!([{
        "hooks": [{ "type": "command", "command": &hook_command }]
    }]);
    hooks["UserPromptSubmit"] = user_prompt_hook;

    // Add Stop hook
    let stop_hook = serde_json::json!([{
        "matcher": "*",
        "hooks": [{ "type": "command", "command": &hook_command }]
    }]);
    hooks["Stop"] = stop_hook;

    // Add Notification hook for permission_prompt (pauses tracking when waiting for approval)
    let notification_hook = serde_json::json!([{
        "matcher": "permission_prompt",
        "hooks": [{ "type": "command", "command": &hook_command }]
    }]);
    hooks["Notification"] = notification_hook;

    // Write updated settings
    let settings_str = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;
    fs::write(&settings_path, settings_str)
        .map_err(|e| format!("Failed to write Claude settings: {}", e))?;

    Ok(())
}

// ============== TAURI COMMANDS ==============

#[tauri::command]
fn check_hooks_installed() -> HooksStatus {
    check_hooks_status()
}

#[tauri::command]
fn install_hooks() -> Result<HooksStatus, String> {
    do_install_hooks()?;
    Ok(check_hooks_status())
}

#[tauri::command]
fn get_projects(state: State<AppState>) -> Result<Vec<Project>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, name, path, color, hourlyRate, createdAt FROM projects ORDER BY name")
        .map_err(|e| e.to_string())?;

    let projects = stmt
        .query_map([], |row| {
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                color: row.get(3)?,
                hourly_rate: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(projects)
}

#[tauri::command]
fn create_project(name: String, path: String, state: State<AppState>) -> Result<Project, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    // Get color based on project count
    let count: i32 = conn
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .unwrap_or(0);

    let colors = [
        "#FF6B6B", "#4ECDC4", "#45B7D1", "#96CEB4", "#FFEAA7", "#DDA0DD", "#98D8C8", "#F7DC6F",
    ];
    let color = colors[count as usize % colors.len()].to_string();

    let project = Project {
        id: generate_id(),
        name,
        path,
        color,
        hourly_rate: None,
        created_at: now_ms(),
    };

    conn.execute(
        "INSERT INTO projects (id, name, path, color, hourlyRate, createdAt) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![project.id, project.name, project.path, project.color, project.hourly_rate, project.created_at],
    )
    .map_err(|e| e.to_string())?;

    Ok(project)
}

#[tauri::command]
fn update_project_rate(project_id: String, hourly_rate: Option<f64>, state: State<AppState>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE projects SET hourlyRate = ?1 WHERE id = ?2",
        params![hourly_rate, project_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn update_project_name(project_id: String, name: String, state: State<AppState>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE projects SET name = ?1 WHERE id = ?2",
        params![name, project_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn delete_project(project_id: String, state: State<AppState>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    // Delete all related data first (foreign key constraints)
    conn.execute("DELETE FROM time_entries WHERE projectId = ?1", params![project_id])
        .map_err(|e| format!("Failed to delete time entries: {}", e))?;
    conn.execute("DELETE FROM active_sessions WHERE projectId = ?1", params![project_id])
        .map_err(|e| format!("Failed to delete active sessions: {}", e))?;
    conn.execute("DELETE FROM invoices WHERE projectId = ?1", params![project_id])
        .map_err(|e| format!("Failed to delete invoices: {}", e))?;
    conn.execute("DELETE FROM projects WHERE id = ?1", params![project_id])
        .map_err(|e| format!("Failed to delete project: {}", e))?;

    Ok(())
}

#[tauri::command]
fn start_tracking(project_id: String, manual_mode: bool, state: State<AppState>) -> Result<ActiveSession, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    // Check if already tracking
    let existing: Option<ActiveSession> = conn
        .query_row(
            "SELECT projectId, startTime, claudeCodeDetected, lastClaudeCheck, manualMode FROM active_sessions WHERE projectId = ?1",
            params![project_id],
            |row| {
                Ok(ActiveSession {
                    project_id: row.get(0)?,
                    start_time: row.get(1)?,
                    claude_code_detected: row.get::<_, i32>(2)? == 1,
                    last_claude_check: row.get(3)?,
                    manual_mode: row.get::<_, i32>(4)? == 1,
                })
            },
        )
        .ok();

    if let Some(mut session) = existing {
        if manual_mode && !session.manual_mode {
            conn.execute(
                "UPDATE active_sessions SET manualMode = 1 WHERE projectId = ?1",
                params![project_id],
            )
            .map_err(|e| e.to_string())?;
            session.manual_mode = true;
        }
        return Ok(session);
    }

    let now = now_ms();
    let session = ActiveSession {
        project_id: project_id.clone(),
        start_time: now,
        claude_code_detected: false,
        last_claude_check: now,
        manual_mode,
    };

    conn.execute(
        "INSERT OR REPLACE INTO active_sessions (projectId, startTime, claudeCodeDetected, lastClaudeCheck, manualMode) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![session.project_id, session.start_time, 0, session.last_claude_check, if manual_mode { 1 } else { 0 }],
    )
    .map_err(|e| e.to_string())?;

    Ok(session)
}

#[tauri::command]
fn stop_tracking(project_id: String, end_time: Option<i64>, state: State<AppState>) -> Result<Option<TimeEntry>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    // Get active session
    let session: Option<ActiveSession> = conn
        .query_row(
            "SELECT projectId, startTime, claudeCodeDetected, lastClaudeCheck, manualMode FROM active_sessions WHERE projectId = ?1",
            params![project_id],
            |row| {
                Ok(ActiveSession {
                    project_id: row.get(0)?,
                    start_time: row.get(1)?,
                    claude_code_detected: row.get::<_, i32>(2)? == 1,
                    last_claude_check: row.get(3)?,
                    manual_mode: row.get::<_, i32>(4)? == 1,
                })
            },
        )
        .ok();

    let session = match session {
        Some(s) => s,
        None => return Ok(None),
    };

    let actual_end_time = end_time.unwrap_or_else(now_ms);

    let entry = TimeEntry {
        id: generate_id(),
        project_id: project_id.clone(),
        start_time: session.start_time,
        end_time: Some(actual_end_time),
        claude_code_active: session.claude_code_detected,
        description: None,
    };

    conn.execute(
        "INSERT INTO time_entries (id, projectId, startTime, endTime, claudeCodeActive, description) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![entry.id, entry.project_id, entry.start_time, entry.end_time, if entry.claude_code_active { 1 } else { 0 }, entry.description],
    )
    .map_err(|e| e.to_string())?;

    conn.execute("DELETE FROM active_sessions WHERE projectId = ?1", params![project_id])
        .map_err(|e| e.to_string())?;

    Ok(Some(entry))
}

#[tauri::command]
fn get_status(state: State<AppState>) -> Result<Status, String> {
    // Refresh caches (before locking db to avoid deadlock)
    {
        let mut cache = state.cache.lock().map_err(|e| e.to_string())?;
        refresh_activity_cache(&mut cache);
        refresh_system_idle_cache(&mut cache);
    }

    // Clone cached data for use in the loop (Arc clone is cheap - just ref count)
    let (cached_entries, cached_idle_time) = {
        let cache = state.cache.lock().map_err(|e| e.to_string())?;
        (Arc::clone(&cache.entries), cache.system_idle_time)
    };

    let conn = state.db.lock().map_err(|e| e.to_string())?;

    let now = now_ms();
    let today_start = get_today_start_ms();
    let week_start = get_week_start_ms();

    // BULK QUERY 1: Get all projects
    let mut stmt = conn
        .prepare("SELECT id, name, path, color, hourlyRate, createdAt FROM projects ORDER BY name")
        .map_err(|e| e.to_string())?;

    let projects: Vec<Project> = stmt
        .query_map([], |row| {
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                color: row.get(3)?,
                hourly_rate: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    // BULK QUERY 2: Get all active sessions at once
    let mut sessions_map: std::collections::HashMap<String, ActiveSession> = std::collections::HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT projectId, startTime, claudeCodeDetected, lastClaudeCheck, manualMode FROM active_sessions")
            .map_err(|e| e.to_string())?;
        let sessions = stmt
            .query_map([], |row| {
                Ok(ActiveSession {
                    project_id: row.get(0)?,
                    start_time: row.get(1)?,
                    claude_code_detected: row.get::<_, i32>(2)? == 1,
                    last_claude_check: row.get(3)?,
                    manual_mode: row.get::<_, i32>(4)? == 1,
                })
            })
            .map_err(|e| e.to_string())?;
        for session in sessions.filter_map(|r| r.ok()) {
            sessions_map.insert(session.project_id.clone(), session);
        }
    }

    // BULK QUERY 3: Get all time aggregates in ONE query
    // Returns: projectId, today_time, week_time, total_time
    let mut time_map: std::collections::HashMap<String, (i64, i64, i64)> = std::collections::HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT projectId,
                    COALESCE(SUM(CASE WHEN startTime >= ?1 THEN endTime - startTime ELSE 0 END), 0) as today_time,
                    COALESCE(SUM(CASE WHEN startTime >= ?2 THEN endTime - startTime ELSE 0 END), 0) as week_time,
                    COALESCE(SUM(endTime - startTime), 0) as total_time
                 FROM time_entries
                 WHERE endTime IS NOT NULL
                 GROUP BY projectId"
            )
            .map_err(|e| e.to_string())?;
        let times = stmt
            .query_map(params![today_start, week_start], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for time in times.filter_map(|r| r.ok()) {
            time_map.insert(time.0, (time.1, time.2, time.3));
        }
    }

    // BULK QUERY 4: Get total claude time (single query)
    let claude_total: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(CASE WHEN endTime IS NULL THEN ?1 - startTime ELSE endTime - startTime END), 0) FROM time_entries WHERE claudeCodeActive = 1",
            params![now],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let mut project_statuses = Vec::new();
    let mut today_total: i64 = 0;

    for project in projects {
        // Get Claude state from activity log (hooks are the source of truth for starting)
        let claude_sessions = get_claude_sessions_for_project_cached(&project.path, &cached_entries);
        let hook_says_active = claude_sessions.iter().any(|(_, state, _)| state == "active");

        // Hooks are source of truth for both display and tracking
        let claude_is_active = hook_says_active;
        let claude_state = if claude_is_active { "active" } else { "stopped" };
        let claude_session_count = if claude_is_active { 1 } else { 0 };

        // Get active session from pre-fetched map
        let active_session = sessions_map.get(&project.id).cloned();
        let manual_mode = active_session.as_ref().map(|s| s.manual_mode).unwrap_or(false);

        // Auto-tracking: start/stop based on Claude activity (only for non-manual sessions)
        let mut session_changed = false;
        if hook_says_active && active_session.is_none() {
            // Hook says active (UserPromptSubmit received) - auto-start tracking
            let _ = conn.execute(
                "INSERT INTO active_sessions (projectId, startTime, claudeCodeDetected, lastClaudeCheck, manualMode) VALUES (?1, ?2, 1, ?2, 0)",
                params![project.id, now],
            );
            session_changed = true;
        } else if active_session.is_some() && !manual_mode {
            // Hooks are source of truth - only stop when hooks say stopped.
            // Process detection is unreliable (pgrep gaps cause flickering).
            // Stale sessions (no hook events for 10 min) are already handled by
            // get_claude_sessions_for_project_cached marking them as "stopped".
            let should_stop = !hook_says_active;
            if should_stop {
                if let Some(ref session) = active_session {
                    let entry_id = uuid::Uuid::new_v4().to_string();
                    let _ = conn.execute(
                        "INSERT INTO time_entries (id, projectId, startTime, endTime, claudeCodeActive, description) VALUES (?1, ?2, ?3, ?4, 1, '')",
                        params![entry_id, project.id, session.start_time, now],
                    );
                    let _ = conn.execute(
                        "DELETE FROM active_sessions WHERE projectId = ?1",
                        params![project.id],
                    );
                    session_changed = true;
                }
            }
        }

        // Only re-fetch if we changed the session
        let final_session = if session_changed {
            conn.query_row(
                "SELECT projectId, startTime, claudeCodeDetected, lastClaudeCheck, manualMode FROM active_sessions WHERE projectId = ?1",
                params![project.id],
                |row| {
                    Ok(ActiveSession {
                        project_id: row.get(0)?,
                        start_time: row.get(1)?,
                        claude_code_detected: row.get::<_, i32>(2)? == 1,
                        last_claude_check: row.get(3)?,
                        manual_mode: row.get::<_, i32>(4)? == 1,
                    })
                },
            )
            .ok()
        } else {
            active_session
        };

        let is_tracking = final_session.is_some();
        let manual_mode = final_session.as_ref().map(|s| s.manual_mode).unwrap_or(false);
        let elapsed_time = final_session.as_ref().map(|s| now - s.start_time).unwrap_or(0);

        // Get times from pre-fetched map (default to 0 if no entries)
        let (today_time, week_time, total_time) = time_map.get(&project.id).copied().unwrap_or((0, 0, 0));
        today_total += today_time;

        project_statuses.push(ProjectStatus {
            project,
            is_tracking,
            manual_mode,
            elapsed_time,
            today_time,
            week_time,
            total_time,
            claude_state: claude_state.to_string(),
            claude_session_count,
        });
    }

    let system_idle_time = cached_idle_time;

    Ok(Status {
        projects: project_statuses,
        today_total,
        claude_total,
        system_idle_time,
    })
}

#[tauri::command]
fn get_entries(project_id: String, day_start: Option<i64>, state: State<AppState>) -> Result<Vec<TimeEntry>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    if let Some(start) = day_start {
        let day_end = start + 86_400_000; // 24 hours in ms
        let mut stmt = conn
            .prepare("SELECT id, projectId, startTime, endTime, claudeCodeActive, description FROM time_entries WHERE projectId = ?1 AND startTime >= ?2 AND startTime < ?3 ORDER BY startTime DESC")
            .map_err(|e| e.to_string())?;

        let entries: Vec<TimeEntry> = stmt.query_map(params![project_id, start, day_end], |row| {
            Ok(TimeEntry {
                id: row.get(0)?,
                project_id: row.get(1)?,
                start_time: row.get(2)?,
                end_time: row.get(3)?,
                claude_code_active: row.get::<_, i32>(4)? == 1,
                description: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        Ok(entries)
    } else {
        let mut stmt = conn
            .prepare("SELECT id, projectId, startTime, endTime, claudeCodeActive, description FROM time_entries WHERE projectId = ?1 ORDER BY startTime DESC")
            .map_err(|e| e.to_string())?;

        let entries: Vec<TimeEntry> = stmt.query_map(params![project_id], |row| {
            Ok(TimeEntry {
                id: row.get(0)?,
                project_id: row.get(1)?,
                start_time: row.get(2)?,
                end_time: row.get(3)?,
                claude_code_active: row.get::<_, i32>(4)? == 1,
                description: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        Ok(entries)
    }
}

#[tauri::command]
fn get_data_path() -> String {
    get_data_dir().to_string_lossy().to_string()
}

#[tauri::command]
fn open_data_folder() -> Result<(), String> {
    let path = get_data_dir();
    Command::new("open")
        .arg(path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn open_invoices_folder() -> Result<(), String> {
    let invoices_dir = invoice::get_invoices_dir();
    Command::new("open")
        .arg(invoices_dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn open_file(file_path: String) -> Result<(), String> {
    Command::new("open")
        .arg(file_path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn delete_entry(entry_id: String, state: State<AppState>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM time_entries WHERE id = ?1", params![entry_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn update_entry(entry_id: String, start_time: i64, end_time: i64, state: State<AppState>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE time_entries SET startTime = ?1, endTime = ?2 WHERE id = ?3",
        params![start_time, end_time, entry_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn add_time_entry(project_id: String, start_time: i64, end_time: i64, state: State<AppState>) -> Result<TimeEntry, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    let entry = TimeEntry {
        id: generate_id(),
        project_id: project_id.clone(),
        start_time,
        end_time: Some(end_time),
        claude_code_active: false,
        description: None,
    };

    conn.execute(
        "INSERT INTO time_entries (id, projectId, startTime, endTime, claudeCodeActive, description) VALUES (?1, ?2, ?3, ?4, 0, NULL)",
        params![entry.id, entry.project_id, entry.start_time, entry.end_time],
    )
    .map_err(|e| e.to_string())?;

    Ok(entry)
}

#[tauri::command]
fn get_weekly_summary(state: State<AppState>) -> Result<WeeklySummary, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    use chrono::{Datelike, Duration, Local};
    let now = Local::now();
    let day_of_week = now.weekday().num_days_from_sunday();
    let days_to_last_sunday = if day_of_week == 0 { 7 } else { day_of_week as i64 };
    let days_to_last_monday = days_to_last_sunday + 6;

    let last_monday = (now.date_naive() - Duration::days(days_to_last_monday))
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .unwrap();

    let last_sunday = (now.date_naive() - Duration::days(days_to_last_sunday))
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_local_timezone(Local)
        .unwrap();

    let last_monday_ms = last_monday.timestamp_millis();
    let last_sunday_ms = last_sunday.timestamp_millis();

    // Get projects with hourly rates
    let mut stmt = conn
        .prepare("SELECT id, name, hourlyRate FROM projects")
        .map_err(|e| e.to_string())?;

    let projects: Vec<(String, String, Option<f64>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let mut summary_projects = Vec::new();
    let mut total_earnings: f64 = 0.0;

    for (project_id, project_name, hourly_rate) in projects {
        let (total_ms, entry_count): (i64, i32) = conn
            .query_row(
                "SELECT COALESCE(SUM(COALESCE(endTime, startTime) - startTime), 0), COUNT(*) FROM time_entries WHERE projectId = ?1 AND startTime >= ?2 AND startTime <= ?3",
                params![project_id, last_monday_ms, last_sunday_ms],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap_or((0, 0));

        if total_ms > 0 {
            let total_hours = (total_ms as f64 / 3600000.0 * 100.0).round() / 100.0;
            let earnings = hourly_rate.map(|rate| (total_hours * rate * 100.0).round() / 100.0);

            if let Some(e) = earnings {
                total_earnings += e;
            }

            summary_projects.push(WeeklySummaryProject {
                project_id,
                project_name,
                total_ms,
                total_hours,
                entry_count,
                hourly_rate,
                earnings,
            });
        }
    }

    Ok(WeeklySummary {
        week_start: last_monday.to_rfc3339(),
        week_end: last_sunday.to_rfc3339(),
        projects: summary_projects,
        total_earnings,
    })
}

// ============== BUSINESS INFO & INVOICE COMMANDS ==============

#[tauri::command]
fn get_business_info(state: State<AppState>) -> Result<BusinessInfo, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    let (name, email, tax_rate): (String, String, f64) = conn
        .query_row(
            "SELECT name, email, taxRate FROM business_info WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| e.to_string())?;

    Ok(BusinessInfo {
        name,
        email: if email.is_empty() { None } else { Some(email) },
        tax_rate,
    })
}

#[tauri::command]
fn save_business_info(
    name: String,
    email: Option<String>,
    tax_rate: f64,
    state: State<AppState>,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    conn.execute(
        "UPDATE business_info SET name = ?1, email = ?2, taxRate = ?3 WHERE id = 1",
        params![name, email.unwrap_or_default(), tax_rate],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}


#[tauri::command]
fn generate_invoice(
    project_id: String,
    start_date: i64,
    end_date: i64,
    extra_hours: f64,
    state: State<AppState>,
) -> Result<String, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    // Get project info
    let (project_name, hourly_rate): (String, Option<f64>) = conn
        .query_row(
            "SELECT name, hourlyRate FROM projects WHERE id = ?1",
            params![project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| e.to_string())?;

    let rate = hourly_rate.ok_or("Project must have an hourly rate set")?;

    // Get business info
    let (business_name, business_email, tax_rate): (String, String, f64) = conn
        .query_row(
            "SELECT name, email, taxRate FROM business_info WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| e.to_string())?;

    if business_name.is_empty() {
        return Err("Please configure your business information in Settings first".to_string());
    }

    // Get time entries for the period
    let mut stmt = conn
        .prepare(
            "SELECT startTime, endTime, description FROM time_entries
             WHERE projectId = ?1 AND startTime >= ?2 AND startTime <= ?3
             ORDER BY startTime ASC",
        )
        .map_err(|e| e.to_string())?;

    let entries_data = stmt
        .query_map(params![project_id, start_date, end_date], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?, row.get::<_, Option<String>>(2)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect::<Vec<_>>();

    if entries_data.is_empty() && extra_hours == 0.0 {
        return Err("No time entries found for this date range and no extra hours provided".to_string());
    }

    // Calculate total hours
    use chrono::{DateTime, Local};
    let mut total_hours = 0.0;

    for (start_time, end_time, _description) in &entries_data {
        let duration_ms = end_time.unwrap_or(*start_time) - start_time;
        let hours = duration_ms as f64 / 3600000.0;
        total_hours += hours;
    }

    // Add extra hours tracked outside of ProTimer
    total_hours += extra_hours;

    // Round to 2 decimal places
    total_hours = (total_hours * 100.0).round() / 100.0;

    // Format date range for the invoice entry
    let start_date_obj = DateTime::from_timestamp_millis(start_date)
        .ok_or("Invalid start date")?
        .with_timezone(&Local);
    let end_date_obj = DateTime::from_timestamp_millis(end_date)
        .ok_or("Invalid end date")?
        .with_timezone(&Local);

    let date_range = format!(
        "{} - {}",
        start_date_obj.format("%b %d, %Y"),
        end_date_obj.format("%b %d, %Y")
    );

    // Create single invoice entry
    let amount = (total_hours * rate * 100.0).round() / 100.0;
    let invoice_entries = vec![invoice::InvoiceEntry {
        date: date_range,
        hours: total_hours,
        rate,
        amount,
    }];

    let subtotal = amount;
    let tax_amount = ((subtotal * tax_rate / 100.0) * 100.0).round() / 100.0;
    let total = ((subtotal + tax_amount) * 100.0).round() / 100.0;

    // Create invoice data
    let invoice_date = Local::now().format("%Y-%m-%d").to_string();

    // Generate filename from date range (e.g., "invoice_2026-02-02_to_2026-02-08.pdf")
    let filename = format!(
        "invoice_{}_to_{}.pdf",
        start_date_obj.format("%Y-%m-%d"),
        end_date_obj.format("%Y-%m-%d")
    );

    // Use date range as invoice "number" (just for display on PDF)
    let invoice_number = format!(
        "{} to {}",
        start_date_obj.format("%b %d, %Y"),
        end_date_obj.format("%b %d, %Y")
    );

    let invoice_data = invoice::InvoiceData {
        invoice_number: invoice_number.clone(),
        invoice_date,
        business_name,
        business_email: if business_email.is_empty() { None } else { Some(business_email) },
        project_name: project_name.clone(),
        entries: invoice_entries,
        subtotal,
        tax_rate,
        tax_amount,
        total,
    };

    // Generate PDF in project-specific folder
    let project_dir = invoice::get_project_invoices_dir(&project_name);
    let output_path = project_dir.join(&filename);

    let pdf_path = invoice::generate_invoice_pdf(invoice_data, output_path)?;

    // Save invoice record to database
    let invoice_id = generate_id();
    conn.execute(
        "INSERT INTO invoices (id, invoiceNumber, projectId, filePath, startDate, endDate, totalAmount, createdAt)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![invoice_id, invoice_number, project_id, pdf_path, start_date, end_date, total, now_ms()],
    )
    .map_err(|e| e.to_string())?;

    Ok(pdf_path)
}

#[tauri::command]
fn get_invoices(state: State<AppState>) -> Result<Vec<InvoiceRecord>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare("SELECT i.invoiceNumber, i.projectId, i.filePath, i.startDate, i.endDate, i.totalAmount, i.createdAt, p.name
                  FROM invoices i
                  LEFT JOIN projects p ON i.projectId = p.id
                  ORDER BY i.createdAt DESC")
        .map_err(|e| e.to_string())?;

    let invoices: Vec<InvoiceRecord> = stmt
        .query_map([], |row| {
            Ok(InvoiceRecord {
                invoice_number: row.get(0)?,
                project_id: row.get(1)?,
                file_path: row.get(2)?,
                start_date: row.get(3)?,
                end_date: row.get(4)?,
                total_amount: row.get(5)?,
                created_at: row.get(6)?,
                project_name: row.get::<_, Option<String>>(7)?.unwrap_or_else(|| "Unknown".to_string()),
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(invoices)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Ensure data directory exists
    let data_dir = get_data_dir();
    fs::create_dir_all(&data_dir).expect("Failed to create data directory");

    // Initialize database
    let db_path = get_db_path();
    let conn = Connection::open(&db_path).expect("Failed to open database");
    init_db(&conn).expect("Failed to initialize database");

    let state = AppState {
        db: Mutex::new(conn),
        cache: Mutex::new(ActivityCache {
            entries: Arc::new(Vec::new()),
            file_modified: None,
            system_idle_time: 0,
            system_idle_checked: 0,
        }),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_projects,
            create_project,
            update_project_rate,
            update_project_name,
            delete_project,
            start_tracking,
            stop_tracking,
            get_status,
            get_entries,
            delete_entry,
            update_entry,
            add_time_entry,
            get_weekly_summary,
            get_data_path,
            open_data_folder,
            open_invoices_folder,
            open_file,
            check_hooks_installed,
            install_hooks,
            get_business_info,
            save_business_info,
            generate_invoice,
            get_invoices,
        ])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Setup file watcher for activity log
            let app_handle = app.handle().clone();
            let activity_log_path = get_activity_log_path();

            // Ensure the activity log file exists
            if !activity_log_path.exists() {
                let _ = fs::File::create(&activity_log_path);
            }

            std::thread::spawn(move || {
                let (tx, rx) = channel();

                let mut watcher = match notify::recommended_watcher(tx) {
                    Ok(w) => w,
                    Err(e) => {
                        eprintln!("Failed to create file watcher: {}", e);
                        return;
                    }
                };

                if let Err(e) = watcher.watch(&activity_log_path, RecursiveMode::NonRecursive) {
                    eprintln!("Failed to watch activity log: {}", e);
                    return;
                }

                loop {
                    match rx.recv() {
                        Ok(Ok(Event { kind: EventKind::Modify(_), .. })) => {
                            // Emit event to frontend when activity log is modified
                            let _ = app_handle.emit("activity-log-changed", ());
                        }
                        Ok(Err(e)) => eprintln!("Watch error: {:?}", e),
                        Err(e) => {
                            eprintln!("Channel error: {:?}", e);
                            break;
                        }
                        _ => {}
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
