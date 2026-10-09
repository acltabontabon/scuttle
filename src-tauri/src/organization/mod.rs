//! Explicit, recoverable organization of loose personal files. Never cleanup.
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::commands::AppState;
use crate::model::Category;
use crate::quarantine::fsx::{self, EntryKind, Identity};
use crate::quarantine::transfer::{self, Ctl};
use crate::safety::paths;
use crate::scanning::{FileEntry, ScanContext};
use crate::{Result, ScuttleError};

const MAX_ITEMS: usize = 1000;
pub const MAX_BATCH: usize = 200;
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn refuse(message: &str) -> ScuttleError {
    ScuttleError::Refused(message.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Screenshots,
    Installers,
}
impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Screenshots => "screenshots",
            Self::Installers => "installers",
        }
    }
    fn category(self) -> Category {
        match self {
            Self::Screenshots => Category::Screenshots,
            Self::Installers => Category::Installers,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grouping {
    Month,
    Together,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opportunity {
    pub id: String,
    pub path: PathBuf,
    pub root: PathBuf,
    pub kind: Kind,
    pub identity: Identity,
    pub app: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub items: Vec<Opportunity>,
    pub partial: bool,
    pub skipped_locations: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Destination {
    pub id: String,
    pub path: PathBuf,
    // Pin the existing ancestor's identity in the review. Ordinary additions to
    // the directory change mtime, so compare object identity, not directory state.
    pub anchor: PathBuf,
    pub identity: Identity,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preference {
    pub destination: Destination,
    pub grouping: Grouping,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedFile {
    pub opportunity: Opportunity,
    pub destination: PathBuf,
    pub note: Option<String>,
    pub cross_volume: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub kind: Kind,
    pub preference: Preference,
    pub items: Vec<PlannedFile>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Pending,
    Moving,
    Moved,
    Failed,
    Cancelled,
    Undoing,
    Undone,
    Attention,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub file: PlannedFile,
    pub status: FileStatus,
    pub after: Option<Identity>,
    pub hash: Option<String>,
    pub note: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Batch {
    pub id: String,
    pub created_unix: i64,
    pub kind: Kind,
    pub preference: Preference,
    pub items: Vec<Record>,
    pub running: bool,
    pub undoing: bool,
    pub revision: u64,
    pub error: Option<String>,
}
#[derive(Default)]
pub struct Runtime {
    pub plans: Mutex<HashMap<String, Plan>>,
    pub active: Mutex<Option<(String, Arc<AtomicBool>)>>,
}
impl Runtime {
    pub fn cancel(&self) {
        if let Some((_, cancel)) = self
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            cancel.store(true, Ordering::SeqCst);
        }
    }
}

/// Default locations are inspected shallowly; explicit root choices still limit
/// them. This is a small bounded probe, never a second recursive filesystem walk.
pub fn discover(state: &AppState, ctx: &ScanContext, roots: &[PathBuf]) -> Result<Inventory> {
    let mut result = Inventory::default();
    let mut destinations = vec![
        state.platform().organization_pictures().join("Screenshots"),
        state.platform().organization_downloads().join("Installers"),
    ];
    for kind in [Kind::Screenshots, Kind::Installers] {
        if let Some(pref) = state
            .store()
            .organization_get::<Preference>("preference", kind.key())?
        {
            destinations.push(pref.destination.path);
        }
    }
    let started = Instant::now();
    let mut steps = 0;
    for root in roots {
        if validate_path(state, root).is_err() {
            result.skipped_locations += 1;
            continue;
        }
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(_) => {
                result.skipped_locations += 1;
                continue;
            }
        };
        let mut paths_found = Vec::new();
        for entry in entries {
            steps += 1;
            if ctx.cancelled() || steps > 10000 || started.elapsed().as_secs() >= 5 {
                result.partial = true;
                break;
            }
            match entry {
                Ok(entry) => paths_found.push(entry.path()),
                Err(_) => result.partial = true,
            }
        }
        paths_found.sort();
        for path in paths_found {
            if ctx.cancelled() || result.items.len() >= MAX_ITEMS {
                result.partial = true;
                break;
            }
            if destinations.iter().any(|d| paths::is_within(&path, d))
                || validate_path(state, &path).is_err()
            {
                continue;
            }
            let Ok(identity) = fsx::identity_of(&path) else {
                result.partial = true;
                continue;
            };
            if identity.kind != EntryKind::File {
                continue;
            }
            let entry = entry(&path, &identity);
            let kind = if crate::detectors::screenshots::named_screenshot(&path) {
                Kind::Screenshots
            } else if crate::detectors::installers::is_installer(&entry, ctx) {
                Kind::Installers
            } else {
                continue;
            };
            let product = crate::detectors::naming::product_key(
                &path.file_stem().unwrap_or_default().to_string_lossy(),
            );
            let app = match ctx.apps.attribute(&product) {
                crate::platform::apps::Attribution::Installed(app) => Some(app.name.clone()),
                _ => None,
            };
            if ctx.ignores.covers(&path, app.as_deref(), kind.category()) {
                continue;
            }
            result.items.push(Opportunity {
                id: id(),
                path,
                root: root.clone(),
                kind,
                identity,
                app,
            });
        }
        if result.partial
            && (ctx.cancelled()
                || steps > 10000
                || started.elapsed().as_secs() >= 5
                || result.items.len() >= MAX_ITEMS)
        {
            break;
        }
    }
    Ok(result)
}
fn entry(path: &Path, identity: &Identity) -> FileEntry {
    FileEntry {
        path: path.to_owned(),
        size: identity.size,
        modified_unix: identity.mtime_ns.map(|n| n / 1_000_000_000),
        accessed_unix: None,
        created_unix: None,
        is_dir: false,
        depth: 1,
        opaque: false,
        name: paths::file_name_lower(path),
        ext: paths::extension(path).unwrap_or_default(),
    }
}

pub fn inventory(state: &AppState) -> Result<Inventory> {
    let mut inventory = state
        .store()
        .organization_get::<Inventory>("inventory", "latest")?
        .unwrap_or_default();
    let ignores = state.store().ignore_set()?;
    inventory.items.retain(|o| {
        !ignores.covers(&o.path, o.app.as_deref(), o.kind.category())
            && fsx::identity_of(&o.path).is_ok_and(|i| i.is_same_state(&o.identity))
    });
    Ok(inventory)
}

/// Check every existing ancestor without following links. Missing tail
/// components are allowed for a proposed destination, never permission errors.
fn validate_path(state: &AppState, path: &Path) -> Result<()> {
    if !path.is_absolute() || paths::normalize(path) != path {
        return Err(refuse("Choose an absolute local folder."));
    }
    if state.protected_paths().is_protected(path) {
        return Err(refuse("Scuttle leaves this location alone."));
    }
    let platform = state.platform();
    if platform
        .application_managed_roots()
        .iter()
        .any(|p| paths::is_within(path, p))
    {
        return Err(refuse("This location belongs to an application."));
    }
    let base = paths::link_check_base(path, Some(&platform.home_dir()));
    for ancestor in path.ancestors().take_while(|p| *p != base) {
        match fsx::identity_of(ancestor) {
            Ok(i) if i.kind == EntryKind::Link => {
                return Err(refuse("A folder on this path is a link. Nothing moved."))
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
        if paths::is_opaque_bundle(ancestor) {
            return Err(refuse("Scuttle does not organize application bundles."));
        }
    }
    Ok(())
}

pub fn register_destination(state: &AppState, path: PathBuf) -> Result<Destination> {
    let path = paths::normalize(&path);
    validate_path(state, &path)?;
    // Structural-folder protection prevents moving the folder itself. A
    // user-selected personal folder can still receive reviewed loose files.
    let personal_folder = state.platform().organization_roots().contains(&path)
        || path == state.platform().organization_pictures();
    if state.protected_paths().is_too_shallow(&path) && !personal_folder {
        return Err(refuse("Choose a folder inside this location."));
    }
    if state
        .platform()
        .install_areas()
        .enclosing(&path, Some(&state.platform().home_dir()))
        .is_some()
    {
        return Err(refuse(
            "Choose a personal folder outside an application installation.",
        ));
    }
    let anchor = path
        .ancestors()
        .find(|p| fsx::identity_of(p).is_ok())
        .ok_or_else(|| refuse("The destination is unavailable."))?
        .to_owned();
    let identity = fsx::identity_of(&anchor)?;
    if identity.kind != EntryKind::Dir {
        return Err(refuse("The destination must be a folder."));
    }
    let destination = Destination {
        id: id(),
        path,
        anchor,
        identity,
    };
    state
        .store()
        .organization_put("destination", &destination.id, &destination)?;
    Ok(destination)
}
pub fn preference(state: &AppState, kind: Kind) -> Result<Preference> {
    if let Some(pref) = state.store().organization_get("preference", kind.key())? {
        return Ok(pref);
    }
    let path = match kind {
        Kind::Screenshots => state.platform().organization_pictures().join("Screenshots"),
        Kind::Installers => state.platform().organization_downloads().join("Installers"),
    };
    Ok(Preference {
        destination: register_destination(state, path)?,
        grouping: if kind == Kind::Screenshots {
            Grouping::Month
        } else {
            Grouping::Together
        },
    })
}
fn check_destination(state: &AppState, destination: &Destination) -> Result<()> {
    validate_path(state, &destination.path)?;
    let now = fsx::identity_of(&destination.anchor)?;
    if !destination.identity.is_same_object(&now) {
        return Err(refuse("The destination folder changed. Review again."));
    }
    if state
        .platform()
        .install_areas()
        .enclosing(&destination.path, Some(&state.platform().home_dir()))
        .is_some()
    {
        return Err(refuse("The destination now belongs to an application."));
    }
    Ok(())
}
fn check_source(state: &AppState, o: &Opportunity) -> Result<()> {
    validate_path(state, &o.path)?;
    if o.path.parent() != Some(o.root.as_path())
        || !state.platform().organization_roots().contains(&o.root)
    {
        return Err(refuse(
            "This file is outside the reviewed loose-file locations.",
        ));
    }
    let settings = state.store().settings()?;
    if !settings.scan_roots.is_empty()
        && !settings
            .scan_roots
            .iter()
            .any(|p| paths::is_within(&o.path, p))
    {
        return Err(refuse("This location is no longer included in scans."));
    }
    if state
        .store()
        .ignore_set()?
        .covers(&o.path, o.app.as_deref(), o.kind.category())
    {
        return Err(refuse("You asked Scuttle to keep this file."));
    }
    if !fsx::identity_of(&o.path)?.is_same_state(&o.identity) {
        return Err(refuse(
            "This file changed. Rummage again before organizing it.",
        ));
    }
    if state
        .platform()
        .install_areas()
        .enclosing(&o.path, Some(&o.root))
        .is_some()
    {
        return Err(refuse("This file belongs to an application installation."));
    }
    Ok(())
}
fn collision_path(path: &Path, reserved: &HashSet<Vec<String>>) -> Result<PathBuf> {
    for n in 0..10000 {
        let candidate = if n == 0 {
            path.to_owned()
        } else {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let ext = path
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            path.with_file_name(format!("{stem} ({n}){ext}"))
        };
        if reserved.contains(&paths::folded_components(&candidate)) {
            continue;
        }
        match std::fs::symlink_metadata(&candidate) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Ok(_) => (),
            Err(e) => return Err(e.into()),
        }
    }
    Err(refuse(
        "Too many files share this name. Choose another destination.",
    ))
}
pub fn plan(
    state: &AppState,
    kind: Kind,
    ids: Vec<String>,
    destination_id: &str,
    grouping: Grouping,
) -> Result<Plan> {
    if ids.is_empty() || ids.len() > MAX_BATCH {
        return Err(refuse("Select between 1 and 200 files to review."));
    }
    if kind == Kind::Installers && grouping != Grouping::Together {
        return Err(refuse("Installers are collected together."));
    }
    let destination = state
        .store()
        .organization_get::<Destination>("destination", destination_id)?
        .ok_or_else(|| refuse("Choose a destination again."))?;
    check_destination(state, &destination)?;
    let available = inventory(state)?;
    let mut seen = HashSet::new();
    let mut reserved = HashSet::new();
    let mut items = Vec::new();
    for id in ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let o = available
            .items
            .iter()
            .find(|o| o.id == id && o.kind == kind)
            .ok_or_else(|| refuse("The selection changed. Rummage again."))?
            .clone();
        let mut folder = destination.path.clone();
        if grouping == Grouping::Month {
            let month = o
                .identity
                .mtime_ns
                .and_then(|n| Local.timestamp_opt(n / 1_000_000_000, 0).single())
                .map(|d| d.format("%Y-%m").to_string())
                .unwrap_or_else(|| "Unknown date".into());
            folder.push(month);
        }
        let original_name = folder.join(
            o.path
                .file_name()
                .ok_or_else(|| refuse("This filename cannot be organized."))?,
        );
        let (target, note) = match (|| -> Result<PathBuf> {
            check_source(state, &o)?;
            validate_path(state, &folder)?;
            if paths::is_within(&o.path, &destination.path) {
                return Err(refuse("This file is already in the destination folder."));
            }
            collision_path(&original_name, &reserved)
        })() {
            Ok(path) => (path, None),
            Err(e) => (original_name, Some(e.to_string())),
        };
        reserved.insert(paths::folded_components(&target));
        let cross_volume = !crate::quarantine::volume::same_volume_hint(&o.path, &target);
        items.push(PlannedFile {
            opportunity: o,
            destination: target,
            note,
            cross_volume,
        });
    }
    let plan = Plan {
        id: id(),
        kind,
        preference: Preference {
            destination,
            grouping,
        },
        items,
    };
    let mut plans = state
        .organization()
        .plans
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if plans.len() >= 16 {
        plans.clear();
    }
    plans.insert(plan.id.clone(), plan.clone());
    Ok(plan)
}

fn save_batch(state: &AppState, batch: &mut Batch) -> Result<()> {
    batch.revision += 1;
    state.store().organization_put("batch", &batch.id, batch)
}
pub fn batch(state: &AppState, id: &str) -> Result<Batch> {
    state
        .store()
        .organization_get("batch", id)?
        .ok_or_else(|| ScuttleError::not_found("organization batch"))
}
pub fn create_batch(state: &AppState, plan_id: &str) -> Result<Batch> {
    let plan = state
        .organization()
        .plans
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(plan_id)
        .ok_or_else(|| refuse("Review this selection again before moving it."))?;
    let latest = state
        .store()
        .organization_get::<Inventory>("inventory", "latest")?
        .unwrap_or_default();
    if plan
        .items
        .iter()
        .any(|i| !latest.items.iter().any(|o| o.id == i.opportunity.id))
    {
        return Err(refuse(
            "A newer rummage replaced this selection. Review again.",
        ));
    }
    if !plan.items.iter().any(|i| i.note.is_none()) {
        return Err(refuse("None of these files can move."));
    }
    let mut batch = Batch {
        id: id(),
        created_unix: chrono::Utc::now().timestamp(),
        kind: plan.kind,
        preference: plan.preference,
        items: plan
            .items
            .into_iter()
            .map(|file| Record {
                status: if file.note.is_some() {
                    FileStatus::Failed
                } else {
                    FileStatus::Pending
                },
                note: file.note.clone(),
                file,
                after: None,
                hash: None,
            })
            .collect(),
        running: true,
        undoing: false,
        revision: 0,
        error: None,
    };
    save_batch(state, &mut batch)?;
    Ok(batch)
}
fn checked_hash(path: &Path, expected: &Identity, cancel: &AtomicBool) -> Result<String> {
    let mut file = fsx::open_no_follow(path)?;
    if !fsx::identity_of_file(&file, path)?.is_same_state(expected) {
        return Err(refuse("This file changed."));
    }
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(refuse("Stopped. The file was left in place."));
        }
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    if !fsx::identity_of_file(&file, path)?.is_same_state(expected) {
        return Err(refuse("This file changed while being checked."));
    }
    Ok(hasher.finalize().to_hex().to_string())
}
fn ensure_parent(state: &AppState, path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| refuse("Missing destination folder."))?;
    validate_path(state, parent)?;
    // Create one component at a time and recheck after every creation.
    let mut missing = Vec::new();
    for p in parent.ancestors() {
        match std::fs::symlink_metadata(p) {
            Ok(meta) if meta.is_dir() => break,
            Ok(_) => return Err(refuse("A destination folder is no longer a directory.")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => missing.push(p),
            Err(e) => return Err(e.into()),
        }
    }
    for p in missing.into_iter().rev() {
        validate_path(state, p)?;
        match std::fs::create_dir(p) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
        validate_path(state, p)?;
    }
    validate_path(state, parent)
}

/// Each row is journaled before moving. Transfer already verifies all copied
/// bytes, publishes without replacement and removes the source last.
pub fn run(state: &AppState, batch: &mut Batch, cancel: &AtomicBool) -> Result<()> {
    run_controlled(state, batch, cancel, None)
}
fn run_controlled(
    state: &AppState,
    batch: &mut Batch,
    cancel: &AtomicBool,
    faults: Option<&dyn transfer::Faults>,
) -> Result<()> {
    let undo = batch.undoing;
    for index in 0..batch.items.len() {
        let status = batch.items[index].status;
        if (undo && status != FileStatus::Moved) || (!undo && status != FileStatus::Pending) {
            continue;
        }
        if cancel.load(Ordering::SeqCst) {
            if !undo {
                batch.items[index].status = FileStatus::Cancelled;
            }
            batch.items[index].note = Some("Stopped. This file was left in place.".into());
            save_batch(state, batch)?;
            continue;
        }
        let result = (|| -> Result<()> {
            let record = &batch.items[index];
            let (from, to, expected) = if undo {
                let expected = record.after.as_ref().ok_or_else(|| {
                    refuse("The moved file could not be verified. Inspect it in its folder.")
                })?;
                (
                    &record.file.destination,
                    &record.file.opportunity.path,
                    expected,
                )
            } else {
                check_source(state, &record.file.opportunity)?;
                check_destination(state, &batch.preference.destination)?;
                (
                    &record.file.opportunity.path,
                    &record.file.destination,
                    &record.file.opportunity.identity,
                )
            };
            let from = from.clone();
            let to = to.clone();
            let expected = expected.clone();
            validate_path(state, &from)?;
            validate_path(state, &to)?;
            if !fsx::identity_of(&from)?.is_same_state(&expected) {
                return Err(refuse(
                    "The file changed or was replaced. It was left in place.",
                ));
            }
            match std::fs::symlink_metadata(&to) {
                Ok(_) => {
                    return Err(refuse(
                        "Another file now occupies the destination. Nothing was overwritten.",
                    ))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
            let hash = checked_hash(&from, &expected, cancel)?;
            if undo && batch.items[index].hash.as_ref() != Some(&hash) {
                return Err(refuse(
                    "The organized file was edited. It was left in place.",
                ));
            }
            ensure_parent(state, &to)?;
            validate_path(state, &from)?;
            validate_path(state, &to)?;
            if !undo {
                check_destination(state, &batch.preference.destination)?;
            }
            batch.items[index].hash = Some(hash);
            batch.items[index].status = if undo {
                FileStatus::Undoing
            } else {
                FileStatus::Moving
            };
            batch.items[index].note = None;
            save_batch(state, batch)?;
            let ctl = Ctl {
                cancel: Some(cancel),
                faults,
                ..Ctl::none()
            };
            transfer::move_file_verified(&from, &to, &expected, &ctl)?;
            let after = fsx::identity_of(&to)?;
            if !undo {
                batch.items[index].after = Some(after);
            }
            batch.items[index].status = if undo {
                FileStatus::Undone
            } else {
                FileStatus::Moved
            };
            Ok(())
        })();
        if let Err(error) = result {
            let record = &mut batch.items[index];
            // A transfer may have succeeded before a metadata/storage error.
            // Do not call it retryable unless its source is still present and
            // the intended destination is absent.
            if matches!(record.status, FileStatus::Moving | FileStatus::Undoing) {
                settle_record(state, record)?;
            } else if !undo {
                record.status = FileStatus::Failed;
            }
            record.note = Some(error.to_string());
        }
        save_batch(state, batch)?;
    }
    batch.running = false;
    save_batch(state, batch)?;
    invalidate_cleanup(state, batch)?;
    if !undo && batch.items.iter().any(|r| r.status == FileStatus::Moved) {
        state
            .store()
            .organization_put("preference", batch.kind.key(), &batch.preference)?;
    }
    Ok(())
}

fn invalidate_cleanup(state: &AppState, batch: &Batch) -> Result<()> {
    let Some(scan) = state.store().latest_scan()? else {
        return Ok(());
    };
    let moved: Vec<&Path> = batch
        .items
        .iter()
        .filter(|r| matches!(r.status, FileStatus::Moved | FileStatus::Undone))
        .map(|r| {
            if r.status == FileStatus::Moved {
                r.file.opportunity.path.as_path()
            } else {
                r.file.destination.as_path()
            }
        })
        .collect();
    for candidate in state.store().candidates_for_scan(&scan.id)? {
        if moved
            .iter()
            .any(|p| *p == candidate.path || candidate.group.iter().any(|m| &m.path == p))
        {
            state.store().forget_candidate(&candidate.id)?;
        }
    }
    Ok(())
}

fn settle_record(state: &AppState, record: &mut Record) -> Result<()> {
    let undo = record.status == FileStatus::Undoing;
    let (from, to, expected) = if undo {
        (
            &record.file.destination,
            &record.file.opportunity.path,
            record.after.as_ref(),
        )
    } else {
        (
            &record.file.opportunity.path,
            &record.file.destination,
            Some(&record.file.opportunity.identity),
        )
    };
    record.status = FileStatus::Attention;
    record.note =
        Some("An interrupted move needs a look. Neither copy was removed during recovery.".into());
    if validate_path(state, from).is_err() || validate_path(state, to).is_err() {
        return Ok(());
    }
    let source = std::fs::symlink_metadata(from);
    let destination = fsx::identity_of(to);
    let missing_source = source
        .as_ref()
        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
    let missing_destination = destination
        .as_ref()
        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
    if missing_source {
        if let Ok(after) = destination {
            if after.kind == EntryKind::File
                && expected.is_some_and(|before| before.is_same_object(&after))
                && record.hash.as_ref().is_some_and(|hash| {
                    checked_hash(to, &after, &AtomicBool::new(false)).is_ok_and(|h| &h == hash)
                })
            {
                if !undo {
                    record.after = Some(after);
                }
                record.status = if undo {
                    FileStatus::Undone
                } else {
                    FileStatus::Moved
                };
                record.note = Some("Recovered after an interrupted move.".into());
            }
        }
    } else if missing_destination
        && expected.is_some_and(|e| fsx::identity_of(from).is_ok_and(|i| e.is_same_state(&i)))
    {
        record.status = if undo {
            FileStatus::Moved
        } else {
            FileStatus::Failed
        };
        record.note =
            Some("The move stopped before completion. The original is still in place.".into());
    }
    Ok(())
}
pub fn reconcile(state: &AppState) -> Result<()> {
    let mut offset = 0;
    let mut interrupted = Vec::new();
    // Collect first: journal writes change the history ordering. Usually there
    // is one interrupted batch; storage failures can leave more than one.
    loop {
        let batches = state
            .store()
            .organization_list::<Batch>("batch", offset, 100)?;
        let count = batches.len();
        if count == 0 {
            break;
        }
        interrupted.extend(batches.into_iter().filter(|b| b.running));
        offset += count;
    }
    for mut batch in interrupted {
        for record in &mut batch.items {
            if matches!(record.status, FileStatus::Moving | FileStatus::Undoing) {
                settle_record(state, record)?;
            } else if record.status == FileStatus::Pending {
                record.status = FileStatus::Cancelled;
                record.note = Some("Stopped when Scuttle closed.".into());
            }
        }
        batch.running = false;
        save_batch(state, &mut batch)?;
        invalidate_cleanup(state, &batch)?;
        if batch.items.iter().any(|r| r.status == FileStatus::Moved) {
            state
                .store()
                .organization_put("preference", batch.kind.key(), &batch.preference)?;
        }
    }
    Ok(())
}

pub fn prepare_undo(state: &AppState, id: &str) -> Result<Batch> {
    let mut batch = batch(state, id)?;
    if !batch.items.iter().any(|r| r.status == FileStatus::Moved) {
        return Err(refuse(
            "There are no unchanged moves to undo in this batch.",
        ));
    }
    batch.undoing = true;
    batch.running = true;
    batch.error = None;
    save_batch(state, &mut batch)?;
    Ok(batch)
}

pub fn open_folder(state: &AppState, id: &str) -> Result<()> {
    let batch = batch(state, id)?;
    let path = &batch.preference.destination.path;
    validate_path(state, path)?;
    if fsx::identity_of(path)?.kind != EntryKind::Dir {
        return Err(refuse("The destination folder is no longer available."));
    }
    state.platform().open_folder(path)
}

pub fn thumbnail(state: &AppState, id: &str) -> Result<Option<String>> {
    use base64::Engine;
    let inventory = inventory(state)?;
    let Some(o) = inventory
        .items
        .iter()
        .find(|o| o.id == id && o.kind == Kind::Screenshots)
    else {
        return Ok(None);
    };
    check_source(state, o)?;
    if o.identity.size > 40 * 1024 * 1024 {
        return Ok(None);
    }
    let mut file = fsx::open_no_follow(&o.path)?;
    if !fsx::identity_of_file(&file, &o.path)?.is_same_state(&o.identity) {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(40 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 40 * 1024 * 1024 {
        return Ok(None);
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16000);
    limits.max_image_height = Some(16000);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let Ok(decoded) = reader.decode() else {
        return Ok(None);
    };
    if !fsx::identity_of_file(&file, &o.path)?.is_same_state(&o.identity) {
        return Ok(None);
    }
    let mut output = std::io::Cursor::new(Vec::new());
    if decoded
        .thumbnail(160, 100)
        .write_to(&mut output, image::ImageFormat::Png)
        .is_err()
    {
        return Ok(None);
    }
    Ok(Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(output.into_inner())
    )))
}

#[cfg(test)]
mod tests;
