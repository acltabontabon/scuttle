# Writing a detector

A detector answers one question and produces **evidence** for its answer. It
does not decide how confident to be, how risky the finding is, what should
happen to it, or whether it may be shown at all. Those are computed centrally,
which is what stops a new detector from being able to do damage.

## The trait

```rust
pub trait Detector: Send {
    fn id(&self) -> &'static str;
    fn category(&self) -> Category;

    /// What Scuttle tells the user it is doing. Must describe real work.
    fn rummaging_note(&self) -> &'static str;

    fn enabled(&self, ctx: &ScanContext) -> bool { true }

    /// Every entry in the shared walk. Cheap: no I/O.
    fn observe(&mut self, entry: &FileEntry, ctx: &ScanContext) {}

    /// Targeted lookups. Check `ctx.cancelled()` in any loop.
    fn probe(&mut self, ctx: &ScanContext, sink: &mut dyn CandidateSink) {}

    /// Deferred expensive work and emission.
    fn finish(&mut self, ctx: &ScanContext, sink: &mut dyn CandidateSink) {}
}
```

Most detectors implement `observe` + `finish` and consume the shared traversal.
**Do not walk the filesystem yourself** unless you genuinely need to look
somewhere specific — seven detectors each walking the disk is seven times the
I/O for the same answers. `probe` exists for the ones that do (known cache
locations, game library manifests).

Put anything expensive in `finish`: hashing, image decoding, grouping. By then
you know which candidates are actually worth the work.

## Emitting a finding

```rust
let finding = Finding::new("installers", Category::Installers, &entry, name)
    .risk(Risk::Low)
    .with(EvidenceKind::InstallerFormat { ext: "dmg".into() })
    .with(EvidenceKind::InstalledAppSupersedes { app: "Google Chrome".into() })
    .with(EvidenceKind::UntouchedFor { days: 184 })
    .saying("Google Chrome is already installed.\n\nThis arrived 184 days ago.");

sink.emit(finding);
```

`risk()` is your *starting position* — how much it would cost to be wrong about
this class of thing — not a conclusion. Evidence can only raise it.

There is no `.confidence()` and no `.action()`. There cannot be.

## Adding evidence kinds

If you need a signal that doesn't exist, add a variant to `EvidenceKind` and
handle it in `describe`. That match is exhaustive, so the compiler forces you
to decide three things:

- **The sentence.** What the user reads. Generated here, from structured data,
  never written into a UI string.
- **The weight.** Positive argues for cleanup, negative against. Be stingy:
  `UntouchedFor` is capped at 25 no matter how old a file is, because age is
  weak evidence and a file is not junk for being old.
- **The risk floor.** Optional, and one-way. `SaveDataDetected` forces
  `Risk::High`, which routes the finding to inspect-only however confident
  everything else was.

Watch for observations that mean different things in different contexts.
"The app is installed" protects an application's cache and *condemns* its
installer, so those are two variants — `ApplicationInstalled` and
`InstalledAppSupersedes` — with opposite weights. Reusing one for both was a
real bug, caught by a test.

Put the signal in evidence, not in a `remark`. Remarks are for voice; evidence
is what gets tested, scored and shown as "why Scuttle noticed".

## Being conservative

A few patterns worth copying:

**A name is not evidence.** A directory called `target`, `build` or `dist`
proves nothing — there are photographers with a folder called `build`. The
developer-debris detector requires a project manifest beside the directory
before it believes the name.

**Back off when someone is using it.** If the owning process is running, or the
project was touched recently, say so as evidence and let the arithmetic do the
rest.

**"Could not tell" is not "no".** `ctx.process_running()` returns
`Option<bool>`. `None` means the process list was unreadable. Emitting
`NoProcessUsingIt` in that case would be a lie — leave the evidence out.

**Ask whose file it is.** A finding the user cannot act on is noise, however
true it is. Two copies of the same JAR in two versions of an IDE's plugin
folder are genuine duplicates and a useless thing to report: removing either
breaks the thing that put them there. Detectors about the user's own files
should skip `ctx.is_application_managed(path)`.

**Do not reclassify something another detector explains.** A path with a cache
rule is a cache, even when that rule is switched off for this scan. Turning off
developer debris once made the Go build cache reappear as a *ghost* —
"go-build is not installed" — which is a worse answer delivered more
confidently. Check `ctx.cache_rule_owns(path)`.

**Say you don't know.** If attribution fails, `Category::Oddments` with
`EvidenceKind::Unclassified` is a better answer than a confident guess. Scuttle
is allowed to shrug.

## Testing

`Harness` runs your detector through the *entire* pipeline — real files in a
temp directory, the real walker, the real safety net — so a test also proves
the walker reached the file and the net let it through.

```rust
#[test]
fn an_installer_for_an_installed_app_is_high_confidence_and_low_risk() {
    let h = Harness::with_apps(&[("Google Chrome", Some("com.google.Chrome"))]);
    let candidates = h.run(
        InstallerDetector::new(),
        vec![fixture_entry("Downloads/Google Chrome.dmg", 200, 220 * 1024 * 1024)],
    );
    let c = one(&candidates);
    assert_eq!(c.risk, Risk::Low);
    assert_eq!(c.confidence, Confidence::High);
    assert_eq!(c.recommended_action, RecommendedAction::Quarantine);
}
```

Write at least three tests:

1. It finds the thing.
2. It does **not** find the lookalike — the photographer's `build` folder, the
   same-size-but-different file, the app that is still installed.
3. It backs off appropriately — save data present, owner running, project
   active.

Use `platform::testing::FixedPlatform` (public API) so your tests don't depend
on what is installed on the machine running them.

## Dry run

The `dry_run` command classifies everything and changes nothing:

```
Would quarantine:
  Google Chrome.dmg          212 MB   installers   high / low
    ✓ A .dmg installer package
    ✓ Google Chrome is already installed
    ✓ Untouched for 184 days

Would review:
  ...

Would surface:
  android-studio-backup.zip  18.7 GB  heavy        medium / high
    ✓ 18.7 GB on disk
    ✗ May contain files you made — it is in Documents

Notes:
  184,233 files looked at, 26 findings, nothing modified.
  12 places could not be read (12 permission, 0 vanished mid-walk).
```

It also reports when the ground truth was unavailable — an unreadable process
list, or no installed applications discovered — because in that state ghost
detection cannot be trusted and you should know before you debug the wrong
thing.

## Registering it

Add it to `detectors::default_set`. If it should be opt-in, gate it on a
setting in `enabled()` the way `DeveloperDebrisDetector` does.
