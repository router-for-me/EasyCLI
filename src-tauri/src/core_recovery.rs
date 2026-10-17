//! Recovery is woken by existing process-status checks, not by a new polling loop.
use super::*;
use std::collections::VecDeque;
use tokio::sync::Notify;

const RETRY_DELAYS: [u64; 3] = [2, 5, 15];
const RETRY_WINDOW: Duration = Duration::from_secs(300);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecoveryStatus {
    pub phase: &'static str,
    pub attempt: usize,
    pub delay_seconds: u64,
    pub detail: String,
}

#[derive(Default)]
struct Policy {
    armed: bool,
    generation: u64,
    attempts: VecDeque<Instant>,
    due: Option<Instant>,
    status: Option<RecoveryStatus>,
}

impl Policy {
    fn running(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.armed = true;
        self.due = None;
        self.status = None;
        // Keep the budget across successful restarts to catch crash loops.
    }

    fn cancel(&mut self) {
        self.running();
        self.armed = false;
        self.attempts.clear();
    }

    fn schedule(&mut self, now: Instant, detail: String) {
        self.attempts
            .retain(|at| now.duration_since(*at) < RETRY_WINDOW);
        let delay = RETRY_DELAYS.get(self.attempts.len()).copied();
        self.due = delay.map(|delay| now + Duration::from_secs(delay));
        self.status = Some(RecoveryStatus {
            phase: if delay.is_some() { "waiting" } else { "failed" },
            attempt: (self.attempts.len() + 1).min(RETRY_DELAYS.len()),
            delay_seconds: delay.unwrap_or(0),
            detail,
        });
        if delay.is_none() {
            self.armed = false;
        }
    }

    fn begin(&mut self, generation: u64, now: Instant) -> bool {
        if !self.armed || self.generation != generation || !self.due.is_some_and(|due| due <= now) {
            return false;
        }
        self.attempts.push_back(now);
        self.due = None;
        if let Some(status) = &mut self.status {
            status.phase = "restarting";
        }
        true
    }
}

#[derive(Default)]
pub(crate) struct CoreRecovery {
    policy: Mutex<Policy>,
    // Healthy status reads only load this flag; they do not lock the policy.
    attention: AtomicBool,
    changed: Notify,
}

impl CoreRecovery {
    pub fn wants_running(&self) -> bool {
        self.policy
            .lock()
            .map(|policy| policy.armed)
            .unwrap_or(false)
    }

    pub fn running(&self) {
        if let Ok(mut policy) = self.policy.lock() {
            policy.running();
            self.attention.store(false, Ordering::Release);
        }
        self.changed.notify_one();
    }

    pub fn cancel(&self) {
        if let Ok(mut policy) = self.policy.lock() {
            policy.cancel();
            self.attention.store(false, Ordering::Release);
        }
        self.changed.notify_one();
    }

    pub fn exited(&self, detail: String) {
        if let Ok(mut policy) = self.policy.lock() {
            if !policy.armed || policy.status.is_some() {
                return;
            }
            eprintln!("CPA kernel exited unexpectedly: {detail}");
            policy.schedule(Instant::now(), detail);
            self.attention.store(true, Ordering::Release);
            self.changed.notify_one();
        }
    }

    pub fn status(&self) -> Option<RecoveryStatus> {
        if !self.attention.load(Ordering::Acquire) {
            return None;
        }
        self.policy.lock().ok()?.status.clone()
    }

    fn halt(&self, detail: String) {
        eprintln!("CPA kernel automatic recovery paused: {detail}");
        if let Ok(mut policy) = self.policy.lock() {
            policy.armed = false;
            policy.due = None;
            policy.status = Some(RecoveryStatus {
                phase: "failed",
                attempt: policy.attempts.len(),
                delay_seconds: 0,
                detail,
            });
            self.attention.store(true, Ordering::Release);
        }
    }

    fn plan(&self) -> Option<(u64, Instant)> {
        let policy = self.policy.lock().ok()?;
        Some((policy.generation, policy.due?))
    }
}

fn publish(app: &tauri::AppHandle) {
    let state = app.state::<CoreProcessState>();
    if let Ok(config) = app.state::<GuiConfigState>().snapshot() {
        if let Ok(status) = current_core_status(Some(state.inner()), Some(config.port)) {
            emit_core_status(app, &status);
        }
    }
}

fn recover(app: &tauri::AppHandle, generation: u64) {
    // Serialize with manual commands and updates. Recheck the generation AFTER
    // acquiring the lock so a queued attempt cannot undo a stop or version switch.
    let Ok(_guard) = CORE_OPERATION_LOCK.lock() else {
        app.state::<CoreProcessState>()
            .recovery
            .halt("Kernel operation lock is poisoned".into());
        publish(app);
        return;
    };
    let state = app.state::<CoreProcessState>();
    if state.is_shutting_down() {
        return;
    }
    let begin = state
        .recovery
        .policy
        .lock()
        .map(|mut policy| policy.begin(generation, Instant::now()))
        .unwrap_or(false);
    if !begin {
        return;
    }
    publish(app);
    let result = app.state::<GuiConfigState>().snapshot().and_then(|config| {
        // Never stop an existing process merely to satisfy a recovery attempt.
        if state.managed_pid().is_some() {
            state.recovery.running();
            return Ok(());
        }
        start_core_process_inner(state.inner(), &config)
    });
    if let Err(error) = result {
        eprintln!("CPA kernel automatic recovery failed: {error}");
        if let Ok(mut policy) = state.recovery.policy.lock() {
            if policy.generation == generation && policy.armed && !state.is_shutting_down() {
                policy.schedule(Instant::now(), error);
            }
        }
    }
    publish(app);
}

pub(crate) fn start(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<CoreProcessState>();
        loop {
            if state.is_shutting_down() {
                return;
            }
            if let Some((generation, due)) = state.recovery.plan() {
                let work = app.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || publish(&work)).await;
                tokio::select! {
                    _ = state.recovery.changed.notified() => continue,
                    _ = tokio::time::sleep_until(tokio::time::Instant::from_std(due)) => {}
                }
                let work = app.clone();
                if let Err(error) =
                    tauri::async_runtime::spawn_blocking(move || recover(&work, generation)).await
                {
                    // A failed worker must not leave an overdue timer spinning.
                    state
                        .recovery
                        .halt(format!("Recovery worker failed: {error}"));
                    let work = app.clone();
                    let _ = tauri::async_runtime::spawn_blocking(move || publish(&work)).await;
                }
            } else {
                state.recovery.changed.notified().await;
                // A crash-loop cutoff has no timer, but still needs an event so
                // hidden windows and the tray receive the final failure state.
                if state
                    .recovery
                    .status()
                    .is_some_and(|status| status.phase == "failed")
                {
                    let work = app.clone();
                    let _ = tauri::async_runtime::spawn_blocking(move || publish(&work)).await;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_loop_keeps_budget_across_successful_restarts() {
        let mut p = Policy::default();
        let mut now = Instant::now();
        p.running();
        for delay in RETRY_DELAYS {
            p.schedule(now, "exit 1".into());
            assert_eq!(p.status.as_ref().unwrap().delay_seconds, delay);
            assert!(!p.begin(p.generation, now));
            now += Duration::from_secs(delay);
            assert!(p.begin(p.generation, now));
            assert!(!p.begin(p.generation, now));
            p.running();
        }
        p.schedule(now, "exit 1".into());
        assert_eq!(p.status.as_ref().unwrap().phase, "failed");
        assert!(!p.armed);
        assert!(p.due.is_none());
    }

    #[test]
    fn stop_or_replacement_invalidates_delayed_attempts() {
        let mut p = Policy::default();
        let now = Instant::now();
        p.running();
        p.schedule(now, "exited".into());
        let old = p.generation;
        p.cancel();
        assert!(!p.begin(old, now + RETRY_WINDOW));
        assert!(p.status.is_none());
        p.running();
        p.schedule(now, "replacement exited".into());
        assert!(!p.begin(old, now + RETRY_WINDOW));
        assert!(p.begin(p.generation, now + Duration::from_secs(2)));
    }

    #[test]
    fn stable_runtime_resets_window_and_startup_failure_is_not_armed() {
        let recovery = CoreRecovery::default();
        recovery.exited("initial startup failed".into());
        assert!(recovery.plan().is_none());
        let mut p = Policy::default();
        let now = Instant::now();
        p.running();
        p.schedule(now, "exit".into());
        p.begin(p.generation, now + Duration::from_secs(2));
        p.running();
        p.schedule(
            now + RETRY_WINDOW + Duration::from_secs(3),
            "later exit".into(),
        );
        assert_eq!(p.status.as_ref().unwrap().attempt, 1);
        assert_eq!(p.status.as_ref().unwrap().delay_seconds, 2);
    }

    #[test]
    fn repeated_observation_does_not_schedule_duplicate_restarts() {
        let recovery = CoreRecovery::default();
        recovery.running();
        recovery.exited("exit 7".into());
        let plan = recovery.plan();
        recovery.exited("exit 7".into());
        assert_eq!(recovery.plan(), plan);
        recovery.cancel();
        recovery.exited("intentional stop".into());
        assert!(recovery.plan().is_none());
        assert!(recovery.status().is_none());
    }

    #[test]
    fn failed_start_attempts_back_off_and_stop_without_waiting_for_another_exit() {
        let mut p = Policy::default();
        let mut now = Instant::now();
        p.running();
        let generation = p.generation;
        p.schedule(now, "unexpected exit".into());
        for delay in RETRY_DELAYS {
            now += Duration::from_secs(delay);
            assert!(p.begin(generation, now));
            p.schedule(now, "port is occupied".into());
        }
        assert_eq!(p.status.as_ref().unwrap().phase, "failed");
        assert_eq!(p.status.as_ref().unwrap().detail, "port is occupied");
        assert!(!p.begin(generation, now + RETRY_WINDOW));
        p.cancel();
        p.running();
        p.schedule(now, "exit after manual restart".into());
        assert_eq!(p.status.as_ref().unwrap().attempt, 1);
    }

    #[test]
    fn worker_failure_removes_due_timer_instead_of_spinning() {
        let recovery = CoreRecovery::default();
        recovery.running();
        recovery.exited("exit".into());
        assert!(recovery.plan().is_some());
        recovery.halt("worker failed".into());
        assert!(recovery.plan().is_none());
        assert!(!recovery.wants_running());
        assert_eq!(recovery.status().unwrap().phase, "failed");
    }
}
