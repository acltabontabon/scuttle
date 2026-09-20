//! Application state: the one place the store, the platform and the running
//! scan are held together.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::model::CleanupCandidate;
use crate::platform::PlatformService;
use crate::quarantine::Quarantine;
use crate::safety::{ActionContext, ProtectedPaths};
use crate::scanning::{
    self, CleanupObserver, Phase, Progress, ScanContext, ScanObserver, ScanOptions, ScanSummary,
};
use crate::space::SpaceOverview;
use crate::storage::{QuarantineRecord, Store};
use crate::{Result, ScuttleError};

/// Shared, cheap to clone, safe to hand to a worker thread.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    store: Arc<Store>,
    platform: Arc<dyn PlatformService>,
    /// `Some` while a rummage is running.
    running: Mutex<Option<RunningScan>>,
    /// The roots of the most recent scan. Nothing outside them can be acted on.
    allowed_roots: Mutex<Vec<PathBuf>>,
}

struct RunningScan {
    id: String,
    cancel: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(platform: Arc<dyn PlatformService>) -> Result<AppState> {
        let store = Arc::new(Store::open(&platform.data_dir().join("scuttle.db"))?);
        std::fs::create_dir_all(platform.quarantine_root())?;

        // Roots default to whatever the platform suggests, so a restart that
        // happens between a scan and an action does not strand the findings.
        let settings = store.settings()?;
        let roots = if settings.scan_roots.is_empty() {
            platform
                .default_scan_roots()
                .into_iter()
                .map(|k| k.path)
                .collect()
        } else {
            settings.scan_roots.clone()
        };

        Ok(AppState {
            inner: Arc::new(Inner {
                store,
                platform,
                running: Mutex::new(None),
                allowed_roots: Mutex::new(roots),
            }),
        })
    }

    pub fn clone_handle(&self) -> AppState {
        self.clone()
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    pub fn platform(&self) -> Arc<dyn PlatformService> {
        Arc::clone(&self.inner.platform)
    }

    pub fn quarantine(&self) -> Result<Quarantine> {
        let retention = self.store().settings()?.quarantine_retention_days;
        Ok(Quarantine::new(
            self.inner.platform.quarantine_root(),
            Arc::clone(&self.inner.store),
            retention,
        ))
    }

    /// Register a new scan, refusing if one is already running.
    pub fn start_scan(&self, options: &ScanOptions) -> Result<String> {
        let mut running = self.lock_running();
        if running.is_some() {
            return Err(ScuttleError::ScanBusy);
        }
        let id = uuid::Uuid::new_v4().to_string();
        *running = Some(RunningScan {
            id: id.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
        });
        drop(running);

        *self.lock_roots() = options.roots.clone();
        self.store().begin_scan(&id, options, super::now_unix())?;
        Ok(id)
    }

    pub fn cancel_scan(&self) -> bool {
        match self.lock_running().as_ref() {
            Some(scan) => {
                scan.cancel.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    fn finish_scan(&self) {
        *self.lock_running() = None;
    }

    /// Run a scan to completion, emitting events as it goes.
    pub fn run_scan(
        &self,
        scan_id: &str,
        options: ScanOptions,
        app: &tauri::AppHandle,
    ) -> Result<ScanSummary> {
        let observer = EventObserver {
            app: app.clone(),
            scan_id: scan_id.to_string(),
        };
        let summary = self.run_scan_with(scan_id, options, &observer)?;
        super::emit_done(app, &summary);
        Ok(summary)
    }

    /// The scan itself, with no dependency on a window.
    ///
    /// Split out from [`AppState::run_scan`] so the whole command-layer path —
    /// start, scan, persist, read back — can be tested without a Tauri handle.
    pub fn run_scan_with(
        &self,
        scan_id: &str,
        options: ScanOptions,
        observer: &dyn ScanObserver,
    ) -> Result<ScanSummary> {
        let cancel = match self.lock_running().as_ref() {
            Some(scan) if scan.id == scan_id => Arc::clone(&scan.cancel),
            // Cancelled or superseded before the worker got going.
            _ => return Err(ScuttleError::ScanBusy),
        };

        let result = (|| {
            let ignores = self.store().ignore_set()?;
            let ctx = ScanContext::new(
                options.clone(),
                Arc::clone(&self.inner.platform),
                ignores,
                cancel,
            );
            let detectors = crate::detectors::default_set(&options);
            let outcome = scanning::run(scan_id, &ctx, detectors, observer);

            self.store().save_candidates(scan_id, &outcome.candidates)?;
            self.store()
                .finish_scan(&outcome.summary, super::now_unix())?;

            let mut settings = self.store().settings()?;
            if !settings.has_rummaged_before {
                settings.has_rummaged_before = true;
                let _ = self.store().save_settings(&settings);
            }

            Ok(outcome.summary)
        })();

        self.finish_scan();
        result
    }

    /// Move a candidate into the drawer, through the safety gate.
    pub fn hold(&self, candidate: &CleanupCandidate) -> Result<QuarantineRecord> {
        let protected = self.protected_paths();
        let roots = self.lock_roots().clone();
        let ctx = ActionContext {
            protected: &protected,
            allowed_roots: &roots,
        };
        self.quarantine()?.hold(candidate, &ctx, super::now_unix())
    }

    /// The id of the most recent completed scan, if there is one.
    pub fn latest_scan_id(&self) -> Option<String> {
        self.store()
            .latest_scan()
            .ok()
            .flatten()
            .map(|scan| scan.id)
    }

    /// Quarantine every member of a group finding except the one to keep.
    /// The Tauri command is a thin wrapper over this.
    pub fn quarantine_group(
        &self,
        id: &str,
        keep: super::KeepChoice,
    ) -> Result<super::GroupOutcome> {
        super::run_group_action(self, id, keep)
    }

    /// Quarantine every confident finding in one pile. The Tauri command is a
    /// thin wrapper over this.
    pub fn quarantine_confident(
        &self,
        category: crate::model::Category,
    ) -> Result<super::BulkOutcome> {
        super::run_bulk_quarantine(self, category)
    }

    pub fn space_overview(&self) -> Result<SpaceOverview> {
        let protected = self.protected_paths();
        crate::space::overview(
            self.inner.platform.as_ref(),
            &protected,
            self.store(),
            &|| false,
        )
    }

    /// Classify everything, change nothing.
    pub fn dry_run(&self, request: super::RummageRequest) -> Result<super::DryRunReport> {
        let settings = self.store().settings()?;
        let roots = request.roots.filter(|r| !r.is_empty()).unwrap_or_else(|| {
            if settings.scan_roots.is_empty() {
                self.inner
                    .platform
                    .default_scan_roots()
                    .into_iter()
                    .map(|k| k.path)
                    .collect()
            } else {
                settings.scan_roots.clone()
            }
        });

        let options = ScanOptions {
            roots,
            include_developer_debris: request
                .include_developer_debris
                .unwrap_or(settings.include_developer_debris),
            heavy_threshold: settings.heavy_threshold,
            ..Default::default()
        };

        let ctx = ScanContext::new(
            options.clone(),
            Arc::clone(&self.inner.platform),
            self.store().ignore_set()?,
            Arc::new(AtomicBool::new(false)),
        );
        let detectors = crate::detectors::default_set(&options);
        let outcome = scanning::run("dry-run", &ctx, detectors, &CleanupObserver);
        Ok(super::dry_run::build(&options, &outcome, &ctx))
    }

    pub fn protected_paths(&self) -> ProtectedPaths {
        let mut protected = ProtectedPaths::for_current_user();
        protected.also_protect("Scuttle's own files", self.inner.platform.data_dir());
        protected
    }

    fn lock_running(&self) -> std::sync::MutexGuard<'_, Option<RunningScan>> {
        self.inner.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_roots(&self) -> std::sync::MutexGuard<'_, Vec<PathBuf>> {
        self.inner
            .allowed_roots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }
}

/// Forwards scan progress to the webview.
struct EventObserver {
    app: tauri::AppHandle,
    scan_id: String,
}

impl ScanObserver for EventObserver {
    fn phase(&self, phase: Phase) {
        super::emit_phase(&self.app, &self.scan_id, phase);
    }
    fn progress(&self, progress: &Progress) {
        super::emit_progress(&self.app, &self.scan_id, progress);
    }
    fn candidate(&self, candidate: &CleanupCandidate) {
        super::emit_found(&self.app, &self.scan_id, candidate);
    }
    fn finished(&self, _summary: &ScanSummary) {
        // `emit_done` is sent by the caller, after the results are saved, so
        // the frontend never asks for findings that are not written yet.
    }
}
