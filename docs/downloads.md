# Hosted downloads

Static release downloads are published with SourceHut Pages at:

```text
https://averagechris.srht.site/slack/
```

Primary installs are Nix-based (`nix run` / `nix profile install` from the
SourceHut flake), but release tarballs are available for manual installs and
checksum verification.

## Build local macOS artifact

On macOS:

```bash
nix build .#release-artifact --out-link result-release-artifact
mkdir -p dist/downloads
cp -p result-release-artifact/* dist/downloads/
```

This creates a deterministic tarball and checksum such as:

```text
dist/downloads/slack-v0.2.0-aarch64-darwin.tar.gz
dist/downloads/slack-v0.2.0-aarch64-darwin.tar.gz.sha256
```

The tarball contains the `slack` binary plus `README.md`, `LICENSE`,
`LICENSE-MIT`, `LICENSE-APACHE`, and `CHANGELOG.md`. It is reproducible:
fixed mtimes, sorted entries, numeric root ownership, and `gzip -n`.

## Build and publish Linux artifact on SourceHut

The SourceHut build manifest builds the Linux artifact, fetches existing
hosted downloads from the current Pages manifest, regenerates the downloads
page, and publishes it with `hut pages publish`. It uses build-scoped OAuth
(`pages.sr.ht/PAGES:RW`) rather than a checked-in token.

The manifest lives at `builds/release-linux-x86_64.yml` (not `.builds/`),
so SourceHut does **not** auto-run it on every push — releases (and Pages
republishes) happen only when the manifest is submitted explicitly.

Submit the release build after `prepare-release` has updated
`builds/release-linux-x86_64.yml` for the new version:

```bash
hut builds submit builds/release-linux-x86_64.yml \
  --note "slack v0.2.0 linux release" \
  --tags "slack/v0.2.0/release" \
  --visibility unlisted
```

The successful job still exposes the Linux tarball and checksum as build
artifacts, but the durable download URLs are the copies published to
SourceHut Pages.

## Build and publish pages

```bash
nix run .#build-pages
nix run .#publish-pages
```

To merge locally-built artifacts with artifacts already hosted on Pages
before publishing:

```bash
nix run .#build-pages -- --include-existing-downloads
```

Defaults:

- domain: `averagechris.srht.site`
- subdirectory: `/slack`

Override if needed:

```bash
nix run .#build-pages -- --domain example.com --subdirectory /slack
nix run .#publish-pages -- --domain example.com --subdirectory /slack
```

The generated pages archive is `dist/pages/slack-pages.tar.gz` and contains
`index.html`, `manifest.json`, and `downloads/`.

## Full release orchestration

The `release` flake app runs the whole pipeline:

```bash
nix run .#release -- --version 0.2.0
```

Steps, in order:

1. `prepare-release` — writes the version to `Cargo.toml` / `Cargo.lock`,
   rewrites the `builds/` manifest artifact filenames, and generates a
   CHANGELOG entry from conventional-commit summaries since the previous
   `vX.Y.Z` tag (using `jj log`).
2. Validation — `nix flake check`, `nix run .#ci-test`,
   `nix run .#ci-clippy` (skippable via `--skip-validate`).
3. Tagging — `nix run .#release-tag` creates `vX.Y.Z` via `jj tag set`
   (git fallback in non-jj checkouts), refuses duplicate tags and empty
   revisions, pushes the tag to origin, then moves the `main` bookmark and
   `jj git push`es it (skippable via `--skip-tag`).
4. Local artifact — `nix build .#release-artifact` copied into
   `dist/downloads/` (skippable via `--skip-artifact`).
5. Pages — `nix run .#build-pages -- --include-existing-downloads`
   (skippable via `--skip-pages`); publish with `--publish-pages`.
6. Linux build — pass `--submit-linux-build` to submit
   `builds/release-linux-x86_64.yml` with `hut builds submit`.

The helper commands above are flake-provided `writeShellApplication`
outputs; there are no standalone release scripts to run directly.
