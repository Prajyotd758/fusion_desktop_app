//! scheduler.rs — Delay + Scheduling for Arceus
//!
//! Cargo.toml:
//!   cron = "0.12"
//!   chrono = { version = "0.4", features = ["serde", "clock"] }
//!   tokio-util = "0.7"
//!
//! Wiring (4 things):
//!   1. `mod scheduler;` and call `scheduler::init(app.handle().clone())` in Tauri `setup`
//!   2. Register commands: get_schedules, cancel_schedule, cancel_pending_cmd
//!   3. In execute_operation add:
//!        Operation::Delay { ms }             => scheduler::delay(ms).await,
//!        Operation::Schedule { when, label, actions } => scheduler::handle_schedule(&app, when, label, actions).await,
//!        Operation::ListSchedules            => scheduler::handle_list(&app).await,
//!        Operation::CancelSchedule { id }    => scheduler::handle_cancel(&app, id).await,
//!   4. Fill the two hooks below (`run_ops`, `speak`) and `is_dangerous`.
//!
//! Rules implemented:
//!   - Missed runs (app/system off) are skipped, never fired late (> LATE_TOLERANCE_SECS).
//!   - Dangerous scheduled ops: warn, 5 min countdown, cancellable, then proceed.
//!   - UI gets `schedules-updated` and `danger-pending` events. Rust owns all state.
//!   - Jobs store resolved ops, so firing needs no LLM call (works offline).
//!   - Cron format is 6 fields: `sec min hour dom mon dow`.

//! scheduler/mod.rs — delay + scheduling for Arceus
//! cargo add cron tokio-util; cargo add chrono --features serde

use std::{
    collections::HashMap,
    str::FromStr,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use chrono::{DateTime, Duration as CDur, Local, NaiveDateTime, TimeZone};
use cron::Schedule;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

use crate::system::types::Operation;

const LATE_TOLERANCE_SECS: i64 = 60;
const TICK_SECS: u64 = 15;
const DANGER_COUNTDOWN_SECS: u64 = 300;
const MAX_INLINE_DELAY_MS: u64 = 30_000;
const FILE_NAME: &str = "schedules.json";

// ───────── Hooks ─────────

fn speak(app: &AppHandle, text: &str, lang: &str) {
    let _ = crate::tts::speak(app, text, lang); // ADJUST path
}

/// Runs stored ops through the normal executor. Stops at first failure.
async fn run_ops(app: &AppHandle, ops: &[Operation], lang: &str) {
    for op in ops {
        if op.action == "delay" {
            delay(op.ms.max(0) as u64).await;
            continue;
        }
        // ADJUST path; execute_operation must be `pub(crate)`
        if let Err(e) = crate::system::llm_ops::execute_operation(app, op, lang) {
            eprintln!("[scheduler] op '{}' failed: {e}", op.action);
            speak(app, &format!("Scheduled task failed: {e}"), lang);
            break;
        }
    }
}

/// Scheduled ops that get a warning + countdown before running.
fn is_dangerous_op(op: &Operation) -> bool {
    matches!(
        op.action.as_str(),
        "shutdown" | "restart" | "sleep" | "delete_file" | "delete_folder"
    )
}

// ───────── Delay ─────────

pub async fn delay(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms.min(MAX_INLINE_DELAY_MS))).await;
}

// ───────── Types ─────────

/// Exactly one field is set by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct When {
    pub in_secs: Option<i64>,
    /// local time, YYYY-MM-DDTHH:MM:SS
    pub at: Option<String>,
    /// 6 fields: sec min hour dom mon dow
    pub cron: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub label: String,
    pub ops: Vec<Operation>,
    pub next_run: DateTime<Local>,
    pub cron: Option<String>, // None = one-shot
    #[serde(default)]
    pub lang: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobView {
    pub id: String,
    pub label: String,
    pub next_run: String,
    pub recurring: Option<String>,
    pub dangerous: bool,
}

static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());
static PENDING: OnceLock<Mutex<HashMap<String, CancellationToken>>> = OnceLock::new();

fn pending() -> &'static Mutex<HashMap<String, CancellationToken>> {
    PENDING.get_or_init(Default::default)
}

// ───────── Persistence ─────────

fn file_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(FILE_NAME))
}

fn load(app: &AppHandle) {
    let Some(p) = file_path(app) else { return };
    if let Ok(s) = std::fs::read_to_string(p) {
        if let Ok(v) = serde_json::from_str::<Vec<Job>>(&s) {
            *JOBS.lock().unwrap() = v;
        }
    }
}

fn save_and_emit(app: &AppHandle) {
    let jobs = JOBS.lock().unwrap().clone();
    if let Some(p) = file_path(app) {
        if let Ok(s) = serde_json::to_string_pretty(&jobs) {
            let _ = std::fs::write(p, s);
        }
    }
    let _ = app.emit("schedules-updated", views(&jobs));
}

fn views(jobs: &[Job]) -> Vec<JobView> {
    let mut v: Vec<JobView> = jobs
        .iter()
        .map(|j| JobView {
            id: j.id.clone(),
            label: j.label.clone(),
            next_run: j.next_run.to_rfc3339(),
            recurring: j.cron.clone(),
            dangerous: j.ops.iter().any(is_dangerous_op),
        })
        .collect();
    v.sort_by(|a, b| a.next_run.cmp(&b.next_run));
    v
}

// ───────── Time helpers ─────────

fn next_cron(expr: Option<&str>) -> Option<DateTime<Local>> {
    Schedule::from_str(expr?).ok()?.upcoming(Local).next()
}

fn resolve_when(w: &When) -> Result<(DateTime<Local>, Option<String>), String> {
    let now = Local::now();
    if let Some(c) = &w.cron {
        let next = next_cron(Some(c)).ok_or("invalid cron expression")?;
        return Ok((next, Some(c.clone())));
    }
    if let Some(s) = w.in_secs {
        if s <= 0 {
            return Err("time must be in the future".into());
        }
        return Ok((now + CDur::seconds(s), None));
    }
    if let Some(a) = &w.at {
        let n = NaiveDateTime::parse_from_str(a, "%Y-%m-%dT%H:%M:%S")
            .map_err(|_| "bad datetime format")?;
        let t = Local
            .from_local_datetime(&n)
            .single()
            .ok_or("ambiguous local time")?;
        if t <= now {
            return Err("time is in the past".into());
        }
        return Ok((t, None));
    }
    Err("no time given".into())
}

fn new_id() -> String {
    format!("{:x}", Local::now().timestamp_nanos_opt().unwrap_or(0))
}

// ───────── Public API (called from execute_operation) ─────────

pub fn add_job(
    app: &AppHandle,
    when: &When,
    label: String,
    ops: Vec<Operation>,
    lang: &str,
) -> Result<JobView, String> {
    if ops.is_empty() {
        return Err("nothing to schedule".into());
    }
    if ops.iter().any(|o| o.action == "schedule") {
        return Err("cannot nest schedules".into());
    }
    let (next_run, cron) = resolve_when(when)?;
    let label = if label.trim().is_empty() {
        "Scheduled task".into()
    } else {
        label
    };
    let job = Job {
        id: new_id(),
        label,
        ops,
        next_run,
        cron,
        lang: lang.to_string(),
    };
    let view = views(std::slice::from_ref(&job)).remove(0);
    JOBS.lock().unwrap().push(job);
    save_and_emit(app);
    Ok(view)
}

pub fn list_jobs() -> Vec<JobView> {
    views(&JOBS.lock().unwrap())
}

pub fn cancel_job(app: &AppHandle, id: &str) -> bool {
    let removed = {
        let mut jobs = JOBS.lock().unwrap();
        let before = jobs.len();
        jobs.retain(|j| j.id != id);
        jobs.len() != before
    };
    if removed {
        save_and_emit(app);
    }
    cancel_pending(Some(id));
    removed
}

/// Voice path: match by label substring, or "all".
pub fn cancel_by_query(app: &AppHandle, q: &str) -> usize {
    let q = q.trim().to_lowercase();
    let ids: Vec<String> = JOBS
        .lock()
        .unwrap()
        .iter()
        .filter(|j| q == "all" || j.label.to_lowercase().contains(&q))
        .map(|j| j.id.clone())
        .collect();
    for id in &ids {
        cancel_job(app, id);
    }
    ids.len()
}

/// Cancel a running danger countdown. None = all (voice "cancel").
pub fn cancel_pending(id: Option<&str>) -> bool {
    let mut p = pending().lock().unwrap();
    match id {
        Some(i) => p.remove(i).map(|t| t.cancel()).is_some(),
        None => {
            let any = !p.is_empty();
            for (_, t) in p.drain() {
                t.cancel();
            }
            any
        }
    }
}

pub fn has_pending() -> bool {
    !pending().lock().unwrap().is_empty()
}

// ───────── Scheduler loop ─────────

pub fn init(app: AppHandle) {
    load(&app);
    // Startup: past-due jobs were missed (system off) -> skip, never fire.
    {
        let now = Local::now();
        let mut jobs = JOBS.lock().unwrap();
        jobs.retain_mut(|j| {
            if (now - j.next_run).num_seconds() <= LATE_TOLERANCE_SECS {
                return true;
            }
            match next_cron(j.cron.as_deref()) {
                Some(n) => {
                    j.next_run = n;
                    true
                }
                None => false,
            }
        });
    }
    save_and_emit(&app);
    tauri::async_runtime::spawn(tick_loop(app));
}

async fn tick_loop(app: AppHandle) {
    let mut t = tokio::time::interval(Duration::from_secs(TICK_SECS));
    loop {
        t.tick().await;
        let now = Local::now();
        let mut due: Vec<Job> = Vec::new();
        let mut changed = false;
        {
            let mut jobs = JOBS.lock().unwrap();
            jobs.retain_mut(|j| {
                if j.next_run > now {
                    return true;
                }
                changed = true;
                if (now - j.next_run).num_seconds() <= LATE_TOLERANCE_SECS {
                    due.push(j.clone());
                }
                match next_cron(j.cron.as_deref()) {
                    Some(n) => {
                        j.next_run = n;
                        true
                    }
                    None => false,
                }
            });
        }
        if changed {
            save_and_emit(&app);
        }
        for j in due {
            let a = app.clone();
            tauri::async_runtime::spawn(async move { fire(a, j).await });
        }
    }
}

async fn fire(app: AppHandle, j: Job) {
    if j.ops.iter().any(is_dangerous_op) {
        let token = CancellationToken::new();
        pending()
            .lock()
            .unwrap()
            .insert(j.id.clone(), token.clone());
        speak(
            &app,
            &format!("{} in 5 minutes. Say cancel to stop it.", j.label),
            &j.lang,
        );
        let _ = app.emit(
            "danger-pending",
            json!({ "id": j.id, "label": j.label, "secs": DANGER_COUNTDOWN_SECS }),
        );
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(DANGER_COUNTDOWN_SECS)) => {}
            _ = token.cancelled() => {
                speak(&app, "Cancelled.", &j.lang);
                let _ = app.emit("danger-cleared", json!({ "id": j.id }));
                return;
            }
        }
        pending().lock().unwrap().remove(&j.id);
        let _ = app.emit("danger-cleared", json!({ "id": j.id }));
    }
    run_ops(&app, &j.ops, &j.lang).await;
}

// ───────── Tauri commands ─────────

#[tauri::command]
pub fn get_schedules() -> Vec<JobView> {
    list_jobs()
}

#[tauri::command]
pub fn cancel_schedule(app: AppHandle, id: String) -> bool {
    cancel_job(&app, &id)
}

#[tauri::command]
pub fn cancel_pending_cmd(id: Option<String>) -> bool {
    cancel_pending(id.as_deref())
}
