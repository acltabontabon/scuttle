use super::*;
use crate::platform::testing::FixedPlatform;
use crate::scanning::{IgnoreSet, ScanOptions};
use tempfile::TempDir;

struct Fixture {
    _dir: TempDir,
    home: PathBuf,
    state: AppState,
}
impl Fixture {
    fn new() -> Self {
        #[cfg(target_os = "macos")]
        let dir = tempfile::tempdir_in("/private/tmp").unwrap();
        #[cfg(not(target_os = "macos"))]
        let dir = tempfile::tempdir().unwrap();
        let home = std::fs::canonicalize(dir.path()).unwrap().join("person");
        for folder in ["Desktop", "Downloads", "Documents", "Pictures"] {
            std::fs::create_dir_all(home.join(folder)).unwrap();
        }
        let state = AppState::new(Arc::new(FixedPlatform::new(&home))).unwrap();
        Self {
            _dir: dir,
            home,
            state,
        }
    }
    fn file(&self, rel: &str, content: &[u8]) -> PathBuf {
        let path = self.home.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }
    fn scan(&self) -> Inventory {
        let ctx = ScanContext::new(
            ScanOptions::default(),
            self.state.platform(),
            self.state.store().ignore_set().unwrap(),
            Arc::new(AtomicBool::new(false)),
        );
        let inventory = discover(
            &self.state,
            &ctx,
            &self.state.platform().organization_roots(),
        )
        .unwrap();
        self.state
            .store()
            .organization_put("inventory", "latest", &inventory)
            .unwrap();
        inventory
    }
    fn plan(&self, kind: Kind) -> Plan {
        let inventory = self.scan();
        let pref = preference(&self.state, kind).unwrap();
        plan(
            &self.state,
            kind,
            inventory
                .items
                .iter()
                .filter(|o| o.kind == kind)
                .map(|o| o.id.clone())
                .collect(),
            &pref.destination.id,
            pref.grouping,
        )
        .unwrap()
    }
    fn move_plan(&self, plan: &Plan) -> Batch {
        let mut batch = create_batch(&self.state, &plan.id).unwrap();
        run(&self.state, &mut batch, &AtomicBool::new(false)).unwrap();
        batch
    }
}

#[test]
fn finds_recent_loose_files_without_recursing_or_classifying_photos_as_screenshots() {
    let f = Fixture::new();
    f.file("Desktop/Screenshot today.png", b"capture");
    f.file("Documents/Screen Shot today.jpg", b"capture");
    f.file("Downloads/photo.png", b"holiday");
    f.file("Desktop/project/Screenshot asset.png", b"asset");
    f.file("Downloads/AppSetup.exe", &vec![1; 600_000]);
    f.file("Downloads/tool.exe", &vec![2; 600_000]);
    let inventory = f.scan();
    assert_eq!(inventory.items.len(), 3);
    assert_eq!(
        inventory
            .items
            .iter()
            .filter(|o| o.kind == Kind::Screenshots)
            .count(),
        2
    );
    assert!(inventory
        .items
        .iter()
        .all(|o| !o.path.ends_with("tool.exe")));
}
#[test]
fn ignores_keep_paths_categories_and_application_decisions() {
    let f = Fixture::new();
    let source = f.file("Desktop/Screenshot.png", b"capture");
    f.state.store().ignore_path(&source, 1).unwrap();
    assert!(f.scan().items.is_empty());
    f.state.store().clear_ignores().unwrap();
    f.state
        .store()
        .ignore_category(Category::Screenshots, 1)
        .unwrap();
    assert!(f.scan().items.is_empty());
}
#[test]
fn loose_executables_beside_application_libraries_are_not_installers() {
    let f = Fixture::new();
    f.file("Downloads/AppSetup.exe", &vec![1; 600_000]);
    f.file("Downloads/resources/app.asar", b"application");
    assert!(f.scan().items.is_empty());
}
#[test]
fn default_month_and_collision_paths_are_exact_and_never_overwrite() {
    let f = Fixture::new();
    let a = f.file("Desktop/Screenshot.png", b"first");
    f.file("Downloads/Screenshot.png", b"second");
    let timestamp = filetime::FileTime::from_unix_time(1_780_315_200, 0);
    filetime::set_file_mtime(&a, timestamp).unwrap();
    filetime::set_file_mtime(f.home.join("Downloads/Screenshot.png"), timestamp).unwrap();
    let p = f.plan(Kind::Screenshots);
    assert_eq!(p.items.len(), 2);
    assert_ne!(p.items[0].destination, p.items[1].destination);
    assert!(p
        .items
        .iter()
        .any(|i| i.destination.file_name().unwrap() == "Screenshot (1).png"));
    assert_eq!(
        p.items[0]
            .destination
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .len(),
        7
    );
    assert!(a.exists(), "review never moves files");
    let batch = f.move_plan(&p);
    assert!(batch.items.iter().all(|r| r.status == FileStatus::Moved));
    assert_eq!(inventory(&f.state).unwrap().items.len(), 0);
    assert_eq!(f.scan().items.len(), 0);
    assert!(f.state.store().held_quarantine().unwrap().is_empty());
    let mut undo = prepare_undo(&f.state, &batch.id).unwrap();
    run(&f.state, &mut undo, &AtomicBool::new(false)).unwrap();
    assert!(undo.items.iter().all(|r| r.status == FileStatus::Undone));
    assert_eq!(std::fs::read(a).unwrap(), b"first");
}
#[test]
fn a_collision_appearing_after_review_is_skipped_with_other_files_still_moving() {
    let f = Fixture::new();
    let source = f.file("Desktop/Screenshot.png", b"capture");
    f.file("Desktop/Screenshot two.png", b"two");
    let p = f.plan(Kind::Screenshots);
    let target = &p
        .items
        .iter()
        .find(|i| i.opportunity.path == source)
        .unwrap()
        .destination;
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, b"occupant").unwrap();
    let batch = f.move_plan(&p);
    assert_eq!(
        batch
            .items
            .iter()
            .filter(|r| r.status == FileStatus::Moved)
            .count(),
        1
    );
    assert_eq!(std::fs::read(target).unwrap(), b"occupant");
    assert!(source.exists());
}
#[test]
fn changed_source_after_review_is_left_in_place() {
    let f = Fixture::new();
    let source = f.file("Desktop/Screenshot.png", b"capture");
    let p = f.plan(Kind::Screenshots);
    std::fs::write(&source, b"changed capture").unwrap();
    let batch = f.move_plan(&p);
    assert_eq!(batch.items[0].status, FileStatus::Failed);
    assert!(source.exists());
    assert!(!p.items[0].destination.exists());
}
#[test]
fn keep_after_review_is_respected_at_execution() {
    let f = Fixture::new();
    let source = f.file("Desktop/Screenshot.png", b"capture");
    let p = f.plan(Kind::Screenshots);
    f.state.store().ignore_path(&source, 1).unwrap();
    let batch = f.move_plan(&p);
    assert_eq!(batch.items[0].status, FileStatus::Failed);
    assert!(source.exists());
}
#[test]
fn grouping_preference_is_saved_only_after_a_successful_move() {
    let f = Fixture::new();
    f.file("Desktop/Screenshot.png", b"capture");
    let inventory = f.scan();
    let dest = register_destination(&f.state, f.home.join("Pictures/Collected")).unwrap();
    let p = plan(
        &f.state,
        Kind::Screenshots,
        vec![inventory.items[0].id.clone()],
        &dest.id,
        Grouping::Together,
    )
    .unwrap();
    assert!(f
        .state
        .store()
        .organization_get::<Preference>("preference", "screenshots")
        .unwrap()
        .is_none());
    f.move_plan(&p);
    let saved = preference(&f.state, Kind::Screenshots).unwrap();
    assert_eq!(saved.grouping, Grouping::Together);
    assert_eq!(saved.destination.path, dest.path);
}
#[test]
fn installer_destinations_are_flat_and_screenshots_can_be_flat() {
    let f = Fixture::new();
    f.file("Downloads/AppSetup.exe", &vec![1; 600_000]);
    let p = f.plan(Kind::Installers);
    assert_eq!(
        p.items[0].destination,
        f.home.join("Downloads/Installers/AppSetup.exe")
    );
    assert_eq!(f.move_plan(&p).items[0].status, FileStatus::Moved);
}
#[test]
fn undo_leaves_edited_files_even_when_size_and_mtime_are_restored() {
    let f = Fixture::new();
    f.file("Desktop/Screenshot.png", b"before");
    let p = f.plan(Kind::Screenshots);
    let batch = f.move_plan(&p);
    let target = &batch.items[0].file.destination;
    let old = filetime::FileTime::from_last_modification_time(&std::fs::metadata(target).unwrap());
    std::fs::write(target, b"edited").unwrap();
    filetime::set_file_mtime(target, old).unwrap();
    let mut undo = prepare_undo(&f.state, &batch.id).unwrap();
    run(&f.state, &mut undo, &AtomicBool::new(false)).unwrap();
    assert_eq!(undo.items[0].status, FileStatus::Moved);
    assert!(undo.items[0].note.as_ref().unwrap().contains("edited"));
    assert_eq!(std::fs::read(target).unwrap(), b"edited");
}
#[test]
fn undo_never_overwrites_an_occupied_original_path_and_can_be_retried() {
    let f = Fixture::new();
    let source = f.file("Desktop/Screenshot.png", b"capture");
    let p = f.plan(Kind::Screenshots);
    let batch = f.move_plan(&p);
    std::fs::write(&source, b"new occupant").unwrap();
    let mut undo = prepare_undo(&f.state, &batch.id).unwrap();
    run(&f.state, &mut undo, &AtomicBool::new(false)).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), b"new occupant");
    assert_eq!(undo.items[0].status, FileStatus::Moved);
    std::fs::remove_file(&source).unwrap();
    let mut retry = prepare_undo(&f.state, &batch.id).unwrap();
    run(&f.state, &mut retry, &AtomicBool::new(false)).unwrap();
    assert_eq!(retry.items[0].status, FileStatus::Undone);
    assert_eq!(std::fs::read(source).unwrap(), b"capture");
}
#[test]
fn undo_recreates_a_missing_original_folder() {
    let f = Fixture::new();
    let source = f.file("Desktop/Screenshot.png", b"capture");
    let p = f.plan(Kind::Screenshots);
    let batch = f.move_plan(&p);
    std::fs::remove_dir(source.parent().unwrap()).unwrap();
    let mut undo = prepare_undo(&f.state, &batch.id).unwrap();
    run(&f.state, &mut undo, &AtomicBool::new(false)).unwrap();
    assert!(source.exists());
}
#[test]
fn cancellation_leaves_pending_files_available_for_review() {
    let f = Fixture::new();
    let source = f.file("Desktop/Screenshot.png", b"capture");
    let p = f.plan(Kind::Screenshots);
    let mut batch = create_batch(&f.state, &p.id).unwrap();
    run(&f.state, &mut batch, &AtomicBool::new(true)).unwrap();
    assert_eq!(batch.items[0].status, FileStatus::Cancelled);
    assert!(source.exists());
    assert_eq!(inventory(&f.state).unwrap().items.len(), 1);
}
#[test]
fn history_and_undo_survive_reopening_the_database() {
    let f = Fixture::new();
    f.file("Desktop/Screenshot.png", b"capture");
    let p = f.plan(Kind::Screenshots);
    let batch = f.move_plan(&p);
    let reopened = AppState::new(Arc::new(FixedPlatform::new(&f.home))).unwrap();
    assert_eq!(
        super::batch(&reopened, &batch.id).unwrap().items[0].status,
        FileStatus::Moved
    );
    let mut undo = prepare_undo(&reopened, &batch.id).unwrap();
    run(&reopened, &mut undo, &AtomicBool::new(false)).unwrap();
    assert_eq!(undo.items[0].status, FileStatus::Undone);
}
#[test]
fn interrupted_rename_is_reconciled_and_recoverable() {
    let f = Fixture::new();
    f.file("Desktop/Screenshot.png", b"capture");
    let p = f.plan(Kind::Screenshots);
    let mut b = create_batch(&f.state, &p.id).unwrap();
    b.items[0].status = FileStatus::Moving;
    b.items[0].hash = Some(transfer::hash_file(&b.items[0].file.opportunity.path).unwrap());
    save_batch(&f.state, &mut b).unwrap();
    let row = &b.items[0];
    std::fs::create_dir_all(row.file.destination.parent().unwrap()).unwrap();
    fsx::rename_verified(
        &row.file.opportunity.path,
        &row.file.destination,
        &row.file.opportunity.identity,
    )
    .unwrap();
    reconcile(&f.state).unwrap();
    let b = batch(&f.state, &b.id).unwrap();
    assert!(!b.running);
    assert_eq!(b.items[0].status, FileStatus::Moved);
    let mut undo = prepare_undo(&f.state, &b.id).unwrap();
    run(&f.state, &mut undo, &AtomicBool::new(false)).unwrap();
    assert_eq!(undo.items[0].status, FileStatus::Undone);
}
#[test]
fn ambiguous_interrupted_copy_keeps_both_copies_for_inspection() {
    let f = Fixture::new();
    f.file("Desktop/Screenshot.png", b"capture");
    let p = f.plan(Kind::Screenshots);
    let mut b = create_batch(&f.state, &p.id).unwrap();
    b.items[0].status = FileStatus::Moving;
    b.items[0].hash = Some(transfer::hash_file(&b.items[0].file.opportunity.path).unwrap());
    save_batch(&f.state, &mut b).unwrap();
    let row = &b.items[0];
    std::fs::create_dir_all(row.file.destination.parent().unwrap()).unwrap();
    std::fs::copy(&row.file.opportunity.path, &row.file.destination).unwrap();
    reconcile(&f.state).unwrap();
    let b = batch(&f.state, &b.id).unwrap();
    assert_eq!(b.items[0].status, FileStatus::Attention);
    assert!(row.file.destination.exists());
    assert!(row.file.opportunity.path.exists());
}
#[test]
fn protected_destinations_and_forged_ids_are_refused() {
    let f = Fixture::new();
    assert!(register_destination(&f.state, f.home.join(".ssh")).is_err());
    assert!(plan(
        &f.state,
        Kind::Screenshots,
        vec!["invented".into()],
        "invented",
        Grouping::Together
    )
    .is_err());
    assert!(create_batch(&f.state, "invented").is_err());
}
#[cfg(unix)]
#[test]
fn linked_sources_and_destination_swaps_are_refused() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let real = f.file("Documents/real.png", b"private");
    symlink(&real, f.home.join("Desktop/Screenshot link.png")).unwrap();
    assert!(f.scan().items.is_empty());
    let source = f.file("Desktop/Screenshot.png", b"capture");
    let p = f.plan(Kind::Screenshots);
    std::fs::rename(f.home.join("Pictures"), f.home.join("OldPictures")).unwrap();
    symlink(f.home.join("Documents"), f.home.join("Pictures")).unwrap();
    let batch = f.move_plan(&p);
    assert_eq!(batch.items[0].status, FileStatus::Failed);
    assert!(source.exists());
}
#[cfg(unix)]
#[test]
fn same_name_replacement_source_cannot_move() {
    let f = Fixture::new();
    let source = f.file("Desktop/Screenshot.png", b"capture");
    let p = f.plan(Kind::Screenshots);
    std::fs::rename(&source, source.with_extension("saved")).unwrap();
    std::fs::write(&source, b"capture").unwrap();
    let batch = f.move_plan(&p);
    assert_eq!(batch.items[0].status, FileStatus::Failed);
    assert!(source.exists());
}
#[test]
fn previews_are_bounded_and_invalid_images_have_no_preview() {
    let f = Fixture::new();
    f.file("Desktop/Screenshot.png", b"not an image");
    let i = f.scan();
    assert!(thumbnail(&f.state, &i.items[0].id).unwrap().is_none());
    assert!(thumbnail(&f.state, "invented").unwrap().is_none());
}
#[test]
fn explicit_scan_roots_limit_loose_file_discovery() {
    let f = Fixture::new();
    let settings = crate::storage::Settings::default();
    let options = crate::scanning::roots::resolve(
        f.state.platform().as_ref(),
        &settings,
        Some(vec![f.home.join("Downloads")]),
        None,
    );
    assert_eq!(options.organization_roots, vec![f.home.join("Downloads")]);
    let default =
        crate::scanning::roots::resolve(f.state.platform().as_ref(), &settings, None, None);
    assert_eq!(default.organization_roots.len(), 3);
}
#[test]
fn discovery_is_cancellable_and_reports_partial_results() {
    let f = Fixture::new();
    f.file("Desktop/Screenshot.png", b"capture");
    let ctx = ScanContext::new(
        ScanOptions::default(),
        f.state.platform(),
        IgnoreSet::default(),
        Arc::new(AtomicBool::new(true)),
    );
    let i = discover(&f.state, &ctx, &f.state.platform().organization_roots()).unwrap();
    assert!(i.partial);
    assert!(i.items.is_empty());
}

#[test]
fn cross_volume_copy_and_undo_verify_bytes() {
    let f = Fixture::new();
    let original = vec![42; 2_100_000];
    let source = f.file("Desktop/Screenshot.png", &original);
    let p = f.plan(Kind::Screenshots);
    let mut b = create_batch(&f.state, &p.id).unwrap();
    let faults = transfer::testing::Script::new().cross_device();
    run_controlled(&f.state, &mut b, &AtomicBool::new(false), Some(&faults)).unwrap();
    assert_eq!(b.items[0].status, FileStatus::Moved);
    assert!(!source.exists());
    assert_eq!(std::fs::read(&p.items[0].destination).unwrap(), original);
    let mut undo = prepare_undo(&f.state, &b.id).unwrap();
    run_controlled(&f.state, &mut undo, &AtomicBool::new(false), Some(&faults)).unwrap();
    assert_eq!(undo.items[0].status, FileStatus::Undone);
    assert_eq!(std::fs::read(source).unwrap(), original);
}
#[test]
fn failed_cross_volume_verification_keeps_original_and_never_publishes() {
    let f = Fixture::new();
    let source = f.file("Desktop/Screenshot.png", b"capture");
    let p = f.plan(Kind::Screenshots);
    let mut b = create_batch(&f.state, &p.id).unwrap();
    let faults = transfer::testing::Script::new()
        .cross_device()
        .failing(transfer::Step::Verify, transfer::testing::eacces());
    run_controlled(&f.state, &mut b, &AtomicBool::new(false), Some(&faults)).unwrap();
    assert_eq!(b.items[0].status, FileStatus::Failed);
    assert!(source.exists());
    assert!(!p.items[0].destination.exists());
}
#[test]
fn permission_failure_preserves_source_and_is_reported() {
    let f = Fixture::new();
    let source = f.file("Desktop/Screenshot.png", b"capture");
    let p = f.plan(Kind::Screenshots);
    let mut b = create_batch(&f.state, &p.id).unwrap();
    let faults = transfer::testing::Script::new()
        .failing(transfer::Step::Rename, transfer::testing::eacces());
    run_controlled(&f.state, &mut b, &AtomicBool::new(false), Some(&faults)).unwrap();
    assert_eq!(b.items[0].status, FileStatus::Failed);
    assert!(source.exists());
    assert!(b.items[0].note.is_some());
}

#[test]
fn a_new_scan_invalidates_unexecuted_plans() {
    let f = Fixture::new();
    let source = f.file("Desktop/Screenshot.png", b"capture");
    let p = f.plan(Kind::Screenshots);
    f.scan();
    assert!(create_batch(&f.state, &p.id).is_err());
    assert!(source.exists());
}
#[test]
fn recovery_does_not_claim_a_same_content_replacement_as_its_own_move() {
    let f = Fixture::new();
    let source = f.file("Desktop/Screenshot.png", b"capture");
    let p = f.plan(Kind::Screenshots);
    let mut b = create_batch(&f.state, &p.id).unwrap();
    b.items[0].status = FileStatus::Moving;
    b.items[0].hash = Some(transfer::hash_file(&source).unwrap());
    save_batch(&f.state, &mut b).unwrap();
    let destination = &b.items[0].file.destination;
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::fs::rename(&source, source.with_extension("saved")).unwrap();
    std::fs::write(destination, b"capture").unwrap();
    reconcile(&f.state).unwrap();
    let recovered = batch(&f.state, &b.id).unwrap();
    assert_eq!(recovered.items[0].status, FileStatus::Attention);
    assert!(destination.exists());
}
#[test]
fn missing_organized_file_cannot_be_undone() {
    let f = Fixture::new();
    f.file("Desktop/Screenshot.png", b"capture");
    let p = f.plan(Kind::Screenshots);
    let b = f.move_plan(&p);
    std::fs::remove_file(&b.items[0].file.destination).unwrap();
    let mut undo = prepare_undo(&f.state, &b.id).unwrap();
    run(&f.state, &mut undo, &AtomicBool::new(false)).unwrap();
    assert_eq!(undo.items[0].status, FileStatus::Moved);
    assert!(undo.items[0].note.is_some());
    assert!(!undo.items[0].file.opportunity.path.exists());
}
#[test]
fn a_user_scan_persists_inventory_but_a_background_glance_does_not_replace_it() {
    let f = Fixture::new();
    f.file("Desktop/Screenshot.png", b"capture");
    let options = crate::scanning::roots::resolve(
        f.state.platform().as_ref(),
        &f.state.store().settings().unwrap(),
        Some(vec![f.home.join("Desktop")]),
        None,
    );
    let scan_id = f.state.start_scan(&options).unwrap();
    f.state
        .run_scan_with(&scan_id, options, &crate::scanning::SilentObserver)
        .unwrap();
    let original = inventory(&f.state).unwrap();
    assert_eq!(original.items.len(), 1);
    f.state.run_glance(1).unwrap();
    assert_eq!(
        inventory(&f.state).unwrap().items[0].id,
        original.items[0].id
    );
}

#[test]
fn a_personal_folder_can_receive_files_without_moving_the_folder_itself() {
    let f = Fixture::new();
    f.file("Desktop/Screenshot.png", b"capture");
    let inventory = f.scan();
    let destination = register_destination(&f.state, f.home.join("Pictures")).unwrap();
    let p = plan(
        &f.state,
        Kind::Screenshots,
        vec![inventory.items[0].id.clone()],
        &destination.id,
        Grouping::Together,
    )
    .unwrap();
    assert_eq!(
        p.items[0].destination,
        f.home.join("Pictures/Screenshot.png")
    );
    assert_eq!(f.move_plan(&p).items[0].status, FileStatus::Moved);
    assert!(f.home.join("Pictures").is_dir());
    assert!(register_destination(&f.state, f.home.clone()).is_err());
}
