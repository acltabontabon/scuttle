# Contributing to Scuttle

Scuttle moves and deletes people's files. That shapes everything below.

## The one rule

> A missed cleanup candidate costs a user some disk space.
> A destructive false positive costs them their thesis.

When those two trade off, the answer is always the same. If you find yourself
writing "this is *probably* safe to remove", the detector needs more evidence,
not more confidence.

## Getting set up

```sh
npm install
npm run app:dev          # run the app
npm test                 # frontend tests
npm run rust:test        # Rust tests
npm run rust:clippy      # Rust lints
npm run typecheck        # TypeScript
```

There's a design workbench at `/preview.html` while the dev server is running.
It mounts the real interface against fixture data so you can look at every
screen — including the awkward ones like "found nothing" and "this folder has
save data in it" — without waiting for a real scan to produce them.

There's also a dry-run mode that classifies everything and changes nothing;
see [docs/writing-a-detector.md](docs/writing-a-detector.md).

## What needs tests

Everything, but these are non-negotiable:

- **Protected paths.** A new rule needs a test that the path is refused, and
  one that a similar-looking path nearby is not.
- **Anything in `safety/`.** Write the test as the attack: the sibling
  directory with a similar name, the `..` that escapes, the symlink that
  redirects, the file that changed after the scan.
- **New detectors.** At minimum: it finds the thing; it does *not* find the
  lookalike; it backs off when the owner is still installed or the project is
  still active.
- **New evidence kinds.** The exhaustive `describe` match in `evidence/mod.rs`
  will force you to decide what the observation is worth. Decide deliberately.

Tests never touch a real home directory. `src-tauri/tests/fixtures.rs` builds
representative trees in a temp directory; extend it rather than reaching for
`$HOME`.

## Adding a detector

See [docs/writing-a-detector.md](docs/writing-a-detector.md). Briefly:

1. Implement `Detector` in `src-tauri/src/detectors/`.
2. Produce `Finding`s with **evidence**. Do not set confidence, risk level or
   the recommended action — you can't, and that's deliberate. The core computes
   them from your evidence.
3. Register it in `detectors::default_set`.
4. Write the three tests above.

If your detector needs a new signal, add a variant to `EvidenceKind` rather
than writing prose into a `remark`. Detection logic belongs in structured data
that can be tested; prose belongs at the edges.

## Colour

Text colours come from the `--ink-*` tokens, or the `--*-ink` accent siblings.
The plain accents (`--honey`, `--moss`, `--clay`) are for fills, dots, bars and
borders; they do not have enough contrast to set type in. `--rule` draws
hairlines, `--hollow` fills recesses — those move in opposite directions
between the two themes, so they are not interchangeable.

`src/styles/tokens.test.ts` reads the stylesheet and checks all of this, so a
palette change that makes something unreadable fails the suite rather than
someone's eyes.

## Voice

Scuttle's personality lives almost entirely in short remarks derived from real
findings. The rules:

- Never manufacture urgency. No exclamation marks, no capitals, no counters of
  "problems".
- Never claim certainty you don't have. "Not sure about this one" is a feature.
- **Never be funny near a destructive action.** Confirmation copy, safety
  notices and refusal messages are plain, specific and serious. There's a test
  that checks the outcome copy for this; there is no test that can check your
  judgement on a new string.

## Pull requests

- One idea per PR.
- `cargo fmt`, `cargo clippy -- -D warnings` and the full test suite pass.
- If the change affects what Scuttle will act on, say so in the description
  and explain what happens when the detector is wrong.

## Code of conduct

Be decent. Assume good faith. Disagreements about a protected-path rule are
usually a sign that the rule needs a comment explaining why it exists.
