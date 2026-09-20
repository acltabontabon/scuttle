//! Representative filesystems, built in a temporary directory.
//!
//! Every tree here is created from scratch inside a `TempDir` and removed when
//! the test ends. Nothing in this suite reads, writes or even walks a real
//! user directory — a test that touched `$HOME` would be exactly the bug this
//! whole project exists to avoid.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use scuttle_core::platform::games::{GameInstall, GameLibrary, LibraryKind};
use scuttle_core::platform::testing::FixedPlatform;
use scuttle_core::platform::PlatformService;
use scuttle_core::safety::ProtectedPaths;
use scuttle_core::scanning::{IgnoreSet, ScanContext, ScanOptions};

pub const MB: u64 = 1024 * 1024;

/// A fake home directory and the platform that describes it.
pub struct World {
    tmp: tempfile::TempDir,
    pub home: PathBuf,
    pub platform: FixedPlatform,
    pub options: ScanOptions,
    pub ignores: IgnoreSet,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> World {
        let tmp = tempfile::tempdir().expect("temp dir");
        let home = tmp.path().join("home/rummager");
        std::fs::create_dir_all(&home).expect("home");
        let platform = FixedPlatform::new(&home);
        World {
            tmp,
            home: home.clone(),
            platform,
            options: ScanOptions {
                roots: vec![home],
                heavy_threshold: 8 * MB,
                duplicate_min_size: 1024,
                ..Default::default()
            },
            ignores: IgnoreSet::default(),
        }
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.home.join(relative)
    }

    pub fn root(&self) -> &Path {
        self.tmp.path()
    }

    /// A file of `size` bytes, last touched `days_old` days ago.
    pub fn file(&self, relative: &str, days_old: i64, size: u64) -> PathBuf {
        self.write(relative, days_old, vec![0u8; size as usize])
    }

    /// A file with exact contents.
    pub fn write(&self, relative: &str, days_old: i64, contents: Vec<u8>) -> PathBuf {
        let path = self.home.join(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, contents).expect("write");
        age(&path, days_old);
        path
    }

    pub fn dir(&self, relative: &str) -> PathBuf {
        let path = self.home.join(relative);
        std::fs::create_dir_all(&path).expect("mkdir");
        path
    }

    pub fn symlink(&self, from: &str, to: &Path) -> PathBuf {
        let link = self.home.join(from);
        std::fs::create_dir_all(link.parent().expect("parent")).expect("mkdir");
        // Unix does not distinguish file and directory symlinks; Windows does.
        #[cfg(unix)]
        std::os::unix::fs::symlink(to, &link).expect("symlink");
        #[cfg(windows)]
        {
            if to.is_dir() {
                std::os::windows::fs::symlink_dir(to, &link).expect("symlink");
            } else {
                std::os::windows::fs::symlink_file(to, &link).expect("symlink");
            }
        }
        link
    }

    /// Build the scan context, with the safety table pointed at the fake home.
    pub fn context(&self) -> ScanContext {
        let platform: Arc<dyn PlatformService> = Arc::new(self.platform.clone());
        let mut ctx = ScanContext::new(
            self.options.clone(),
            platform,
            self.ignores.clone(),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        let mut protected = ProtectedPaths::for_home(&self.home);
        protected.also_protect("Scuttle's own files", self.home.join(".scuttle"));
        ctx.protected = Arc::new(protected);
        ctx
    }
}

/// Backdate a file so age-based evidence has something to work with.
pub fn age(path: &Path, days: i64) {
    let when = std::time::SystemTime::now()
        - std::time::Duration::from_secs((days.max(0) * 86_400) as u64);
    let times = std::fs::FileTimes::new()
        .set_accessed(when)
        .set_modified(when);
    let file = std::fs::File::options()
        .write(true)
        .open(path)
        .expect("open");
    file.set_times(times).expect("set times");
}

// ---------------------------------------------------------------------------
// The trees
// ---------------------------------------------------------------------------

/// A machine with nothing wrong with it. The most important fixture: a scan of
/// this must come back quiet.
pub fn healthy_system() -> World {
    let world = World::new();
    world.file("Documents/notes.md", 3, 4096);
    world.file("Documents/thesis.docx", 1, 2 * MB);
    world.file("Desktop/todo.txt", 0, 200);
    world.file("Downloads/receipt.pdf", 12, 300 * 1024);
    world.dir("Library/Application Support");
    world.file(
        "Library/Application Support/com.example.editor/Cache/a.cache",
        2,
        512 * 1024,
    );
    world
}

/// A game that was uninstalled, with a shader cache left behind — and, next to
/// it, one that still has save data in it.
pub fn abandoned_game() -> World {
    let mut world = World::new();
    let common = world.path("Steam/steamapps/common");
    std::fs::create_dir_all(&common).expect("mkdir");

    world.file("Steam/steamapps/common/Hades/game.pak", 20, 30 * MB);

    // A real shader cache is hundreds of small blobs, not one big file; the
    // "is this generated content" rule counts files, so the fixture has to be
    // shaped like the thing it stands in for.
    for n in 0..8 {
        world.file(
            &format!("Steam/steamapps/common/Cyberpunk 2077/shadercache/{n}.bin"),
            300,
            5 * MB,
        );
    }
    world.file(
        "Steam/steamapps/common/Cyberpunk 2077/logs/run.log",
        300,
        MB,
    );

    for n in 0..6 {
        world.file(
            &format!("Steam/steamapps/common/Old RPG/shadercache/{n}.bin"),
            400,
            5 * MB,
        );
    }
    world.file(
        "Steam/steamapps/common/Old RPG/saves/slot1.sav",
        400,
        64 * 1024,
    );

    // A live machine always has *some* process list. An empty one means
    // "could not tell", which detectors must handle differently.
    world.platform = world.platform.clone().with_process("finder");
    world.platform = world.platform.clone().with_library(GameLibrary {
        kind: LibraryKind::Steam,
        root: world.path("Steam"),
        install_roots: vec![common],
        generated_roots: vec![],
        installed: vec![GameInstall {
            name: "Hades".into(),
            id: "1145360".into(),
            install_dir: Some(world.path("Steam/steamapps/common/Hades")),
        }],
    });
    world
}

/// Downloads full of installers, including five copies of the same one.
pub fn old_installers() -> World {
    let mut world = World::new();
    world.file("Downloads/Google Chrome.dmg", 200, 12 * MB);
    for n in 1..=4 {
        world.file(
            &format!("Downloads/Chrome ({n}).dmg"),
            250 + n as i64 * 10,
            12 * MB,
        );
    }
    world.file("Downloads/ObscureTool.dmg", 40, 9 * MB);
    world.file("Downloads/notes.txt", 5, 1024);
    world.platform = world
        .platform
        .clone()
        .with_app("Google Chrome", Some("com.google.Chrome"));
    world
}

/// Byte-identical files scattered around, plus same-size decoys.
pub fn duplicate_files() -> World {
    let world = World::new();
    let payload: Vec<u8> = (0..(200 * 1024)).map(|i| (i % 251) as u8).collect();
    let mut decoy = payload.clone();
    // Differs only in the middle, where a head/tail probe cannot see it.
    decoy[100 * 1024] ^= 0xff;

    world.write("Downloads/report.pdf", 200, payload.clone());
    world.write("Downloads/archive/report.pdf", 400, payload.clone());
    world.write("Work/report-final.pdf", 30, payload);
    world.write("Downloads/decoy.pdf", 200, decoy);
    world
}

/// Places that must never appear in results, however tempting they look.
pub fn dangerous_paths() -> World {
    let world = World::new();
    world.file(".ssh/id_ed25519", 900, 4096);
    world.file(".ssh/known_hosts", 900, 8192);
    world.file(".gnupg/secring.gpg", 900, 64 * 1024);
    world.file(".aws/credentials", 900, 1024);
    world.file("Downloads/vault.kdbx", 900, 40 * MB);
    world.file("Downloads/server.pem", 900, 4096);
    world.file("Dropbox/taxes/2019.pdf", 900, 20 * MB);
    world.file("code/project/.git/objects/ab/cdef", 900, 30 * MB);
    world.file("Library/Keychains/login.keychain-db", 900, 10 * MB);
    world
}

/// A real project with a stale build directory, and a photographer's folder
/// that merely happens to be called `build`.
pub fn developer_project() -> World {
    let world = World::new();
    world.file("code/thing/Cargo.toml", 400, 512);
    world.file("code/thing/src/main.rs", 400, 2048);
    world.file("code/thing/target/debug/binary", 400, 200 * MB);

    world.file("code/live/package.json", 1, 512);
    world.file("code/live/node_modules/dep/index.js", 1, 150 * MB);

    world.file("Photos/build/shoot-001.raw", 400, 200 * MB);
    world
}

/// A symlink that points at an ancestor, and one that points outside the tree.
pub fn symlink_loop() -> World {
    let world = World::new();
    world.file("deep/a/b/c/real.txt", 10, 1024);
    world.symlink("deep/a/b/c/back", &world.home);

    let outside = world.root().join("elsewhere");
    std::fs::create_dir_all(&outside).expect("mkdir");
    std::fs::write(outside.join("secret.txt"), "not yours").expect("write");
    world.symlink("deep/escape", &outside);
    world
}
