# The website

<https://acltabontabon.com/scuttle/> — what Scuttle is, what each operating
system will say about an unsigned download, and where to get one.

It lives in [`www/`](../www), separate from the application on purpose: the
desktop app ships React and the Tauri API, and none of that belongs in a page
whose job is four paragraphs and a download button. `www/` has its own
`package.json`, its own lockfile and one dependency, which is Vite.

## Working on it

```sh
npm --prefix www install
npm --prefix www run dev        # http://localhost:5280
```

The page is plain HTML, one stylesheet and two small modules. There is no
framework and no router.

- `www/index.html` — all of the copy
- `www/src/styles.css` — the tokens are copied from the application's own
  `src/styles/tokens.css`, so the site and the product are recognisably the
  same object. If a colour changes there, change it here too.
- `www/src/main.js` — the piles, the drawer, the demo player, the reveals
- `www/src/downloads.js` — resolving the latest release
- `www/scripts/media.mjs` — copies `docs/media/` into the build

Everything JavaScript does here is an improvement on a page that already
works. The download buttons are real links in the markup, the drawer's
contents are an ordinary image in the document, and the piles are decoration.
Turn scripting off and the page still explains Scuttle and still gets you to a
download.

## Checking it the way it will actually be served

The site is served from `/scuttle/`, not from the root. `base` is relative so
that should not matter — and this is how you find out that it does not:

```sh
npm --prefix www run preview    # http://localhost:4173/scuttle/
```

That builds, stages the output one directory deep, and serves it, so every
asset URL is exercised at the path GitHub Pages will use.

Worth looking at each time: both themes (the site follows
`prefers-color-scheme`), a phone width, tabbing through with the keyboard, and
the page with reduced motion on — where the piles are simply drawn, the
sections are already in place, and the drawer arrives open instead of sliding.

## Downloads

`downloads.js` asks the GitHub API for the latest *stable* release when
someone opens the page, then points the buttons at its assets. No token, no
backend.

It degrades in every direction:

- **No release yet** — says so, and links to the repository. It does not offer
  a download that would 404.
- **Only prereleases** — treated the same as no release. `/releases/latest`
  excludes them, which is exactly what is wanted.
- **Offline, blocked or rate-limited** — the markup's own links to the
  releases page stay as they are.
- **A release missing one of the three assets** — that row points at the
  release page and says the build is not in this version.

The operating system is guessed from the user agent to decide which row to
lead with, and every row stays visible and clickable. A browser cannot
reliably say whether a Mac is Apple silicon or Intel — Chrome reports Intel
under Rosetta — so the Mac default is Apple silicon, labelled in words, with
Intel one click away.

Asset names come from the release workflow, so the two have to agree:
`Scuttle-<version>-macos-apple-silicon.dmg`, `-macos-intel.dmg`,
`-windows-x64-setup.exe`. If [`releasing.md`](releasing.md) ever changes them,
change `TARGETS` in `downloads.js` too.

## Media

The screenshots and the demo are in `docs/media/`, where the README and the
release notes already point at them, and are copied into the site at build
time. One copy, one place to update.

[`../scripts/demo/record.sh`](../scripts/demo/record.sh) makes them by driving
the real application against an invented home directory. Nothing on the site
is a mock-up.

## Deploying

`.github/workflows/pages.yml`, on a push to `main` that touches `www/` or
`docs/media/`, or by hand from the Actions tab. It does not run on a release
tag — the page resolves the latest release when someone opens it, so a new
release reaches the site without a rebuild.

The application's own workflows ignore `www/`, `docs/` and Markdown on pushes
to `main`, so fixing a typo here does not rebuild two DMGs and an installer.

### The domain, which is shared

`acltabontabon.com` belongs to the **`acltabontabon.github.io`** repository,
which is where its `CNAME` file lives. GitHub applies a user site's custom
domain to every project site on the same account, so this repository gets
`/scuttle/` from its own name and needs nothing else.

That means, and this matters:

- **Do not add a `CNAME` file to this repository**, and do not set a custom
  domain on it in Settings. Either would try to take the apex from the root
  site.
- No DNS change is needed. No change to the root site is needed.
- `acltabontabon.github.io/scuttle/` redirects to `acltabontabon.com/scuttle/`
  on its own.

### One-off setup

On the repository: **Settings → Pages → Source: GitHub Actions**. Until that
is done the workflow will fail at the deploy step. Nothing else is required —
there are no secrets, and `GITHUB_TOKEN` covers the rest.
