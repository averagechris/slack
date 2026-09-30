# Release process

Future slack releases are GitHub-only. The SHA-pinned Fleet Rust preset
prepares and validates the release commit, then atomically publishes `main` and
its annotated `vX.Y.Z` tag to `averagechris/slack`. The tag starts the read-only
workflow in `.github/workflows/release.yml`, which builds `aarch64-darwin` on
`macos-14` and `x86_64-linux` on `ubuntu-24.04`.

Each Actions artifact contains one `slack` archive, its checksum sidecar, and a
`release-identity-<platform>` file. A successful run therefore produces six
files. Identity files are verification evidence, not release assets: publish
only the two archives and two checksum sidecars.

## Prepare and build

```bash
nix run .#release -- --version X.Y.Z --check
nix run .#release -- --version X.Y.Z
```

Run both commands from a fresh empty `@` whose parent exactly matches local
`main` and `main@origin`. `--check` is a nonmutating ref/version preflight. The
real command updates `Cargo.toml`, `Cargo.lock`, and `CHANGELOG.md`, runs the
configured release gates, verifies the local artifact, creates an annotated
tag, and atomically pushes the release commit and tag. It does not create a
GitHub Release, upload assets, or dispatch the downloads site.

The workflow does not run on a `main` push. To recover an existing release,
dispatch `release.yml` from `main` with its existing tag. The tag must match
`vX.Y.Z`, be annotated on `origin`, and peel to an ancestor of selected `main`.
Tag pushes require the event SHA to equal the peeled commit. Forks, pull
requests, malformed or lightweight tags, and mismatched refs fail closed.

## Publish the verified artifacts

Use an authorized local `gh` session and a new empty working directory.

1. Inspect the successful run in `averagechris/slack`. Require workflow name
   `Build release artifacts (manual publication required)`, conclusion
   `success`, and either a tag-push run at the peeled tag commit or a recovery
   dispatch from `main`.
2. Resolve `repos/averagechris/slack/git/ref/tags/vX.Y.Z` with `gh api`. Require
   its object type to be `tag`; fetch that tag object, require its target type
   to be `commit`, and record both object IDs.
3. Download Actions artifacts `release-aarch64-darwin` and
   `release-x86_64-linux`. Require exactly these six files and no others:

   ```text
   slack-vX.Y.Z-aarch64-darwin.tar.gz
   slack-vX.Y.Z-aarch64-darwin.tar.gz.sha256
   release-identity-aarch64-darwin
   slack-vX.Y.Z-x86_64-linux.tar.gz
   slack-vX.Y.Z-x86_64-linux.tar.gz.sha256
   release-identity-x86_64-linux
   ```

4. Check each sidecar with `shasum -a 256 -c`. Require each identity file to
   contain exactly two lines: the recorded tag object ID, then the peeled
   commit ID.
5. List **all** releases, including drafts, with the paginated releases API and
   select by exact `tag_name`. If none exists, resolve and compare the tag IDs
   again, then create a draft using `gh release create --verify-tag --draft
   --notes-from-tag`. Upload only the four archives and sidecars, without
   `--clobber`.
6. If one draft exists, list all of its assets through the API. Stop if it is
   public, if multiple releases match, or if any unexpected asset name exists.
   Download every present asset and compare it byte-for-byte with the matching
   verified local file. Upload only missing expected assets, without clobbering.
7. Download the complete draft into another new empty directory. Require
   exactly the four expected names and compare all four remote files
   byte-for-byte with the local sources. Check both downloaded sidecars again.
   For every asset, require the API `digest` to equal
   `sha256:$(shasum -a 256 "$file" | cut -d' ' -f1)`.
8. Resolve the remote annotated tag again and require both IDs to match the
   recorded values. Only then undraft with `gh release edit vX.Y.Z
   --draft=false`.
9. Dispatch the downloads site with the exact project key (the Fleet registry
   row is `slack-rs`, but the Pages subdirectory and artifact prefix are
   `slack`):

   ```bash
   gh workflow run pages.yml --repo averagechris/averagechris.github.io \
     -f project=slack -f tag=vX.Y.Z -f sha="$commit"
   ```

   Wait for the site workflow, then verify the live latest version, both
   download URLs, and displayed checksums.

## Historical SourceHut releases

The SourceHut Pages content, local Pages apps, and
`builds/release-linux-x86_64.yml` are archival records. Do not submit the
manifest, upload future artifacts to SourceHut, or dual-publish future tags.
There is no LFS, crates.io, or Homebrew release step.

Versions are plain semver `X.Y.Z`; release tags are annotated `vX.Y.Z`. Never
create or push a release tag merely to test this process.
