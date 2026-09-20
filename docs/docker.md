# Docker

**Scuttle does not publish a container image, and this is the reasoning rather
than an oversight.**

The question was worth asking properly, because a sibling project does ship one
and the release setup here was adapted from it. But Draft Canvas is a static
web app: its image is nginx plus a `dist/` directory, and running it in a
container is the same product doing the same thing somewhere else. Nothing about
Scuttle maps onto that.

## What was considered

**A headless diagnostic CLI.** `scuttle --dry-run` is genuinely useful and
genuinely headless — it classifies everything, prints the evidence, and changes
nothing. It is the closest thing to a container target here. Two things stop it:

- It is a subcommand of the desktop binary, which links Tauri. Building it for
  Linux means building against `webkit2gtk` and friends to produce something
  that never opens a window. Splitting a headless binary out of `scuttle_core`
  is possible — the IPC layer is one module — but it is a feature, not
  packaging.
- More decisively, `platform::unsupported` is a stub. It knows nothing about
  where a Linux system keeps applications, caches or games, and
  `installed_apps()` returns an empty list, which the dry run itself warns
  about: *"No installed applications discovered — ghost detection is
  untrustworthy here."* An image that scanned a mounted directory would report
  confident nonsense. There is also no way to point a scan at an arbitrary path
  from the command line; roots come from the platform layer.

  So a useful diagnostic image needs a real Linux platform implementation
  first. That is worth doing, and it is a different piece of work.

**A demo in a browser.** The frontend alone is not Scuttle. Every screen it
draws comes from the Rust core over IPC, and served on its own it renders
`Unavailable` everywhere. There is a design workbench at `/preview.html` with
fixed fixture data, but it is a tool for looking at components — its Rummage
button does nothing and its drawer moves no files. Publishing that as a demo
would be presenting a mock-up as the product, which is worse than publishing
nothing.

**Anything else.** A container that reached real files would need the host
filesystem mounted into it, which is exactly the shape — broad mounts, a
process deciding what to delete, a privilege boundary that is not really there —
that Scuttle's entire safety model exists to avoid. A web service that deleted
files on request is not a smaller version of Scuttle; it is a different and much
worse idea.

## If this changes

The precondition is a real Linux platform layer: scan roots, application
discovery, protected paths and a quarantine root that mean something on Linux.
With that, a read-only diagnostic image becomes reasonable — a scoped `-v
/some/path:/scan:ro` mount, no privileged mode, no Docker socket, no host root,
reporting and never writing. Until then there is nothing honest to put in an
image.

None of this affects the native releases, which are the actual distribution:
see [`releasing.md`](releasing.md).
