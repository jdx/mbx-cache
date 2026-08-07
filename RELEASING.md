# Releasing mise-cache

Releases are managed by release-plz. It derives versions from Git tags, updates
`Cargo.toml` and `CHANGELOG.md` in a release PR, and never publishes the service
to crates.io.

## Repository setup

Create a `RELEASE_PLZ_TOKEN` Actions secret containing a fine-grained GitHub
token for this repository with read/write access to contents and pull requests.
A token distinct from `GITHUB_TOKEN` is required so release-plz's pull requests
trigger the normal CI and review workflows.

## Normal release flow

1. A push to `main` opens or updates the release-plz release PR.
2. The daily release job enables auto-merge when the previous release is at
   least seven days old and a `feat` or `fix` commit is pending. The first
   release is allowed immediately. Manually dispatch `auto-merge-release` to
   bypass the cadence checks.
3. Merging the release PR creates a `vX.Y.Z` tag and draft GitHub release.
4. The release workflow publishes the multi-platform image, uploads its
   immutable reference as `container-image.txt`, and publishes the release.

The image reference in `container-image.txt` is the value to use for
`MISE_CACHE_IMAGE` in the OVH deployment.

## Recovery

If image publishing fails after a tag exists, manually dispatch the `release`
workflow with that tag. It rebuilds the image, replaces `container-image.txt`,
and publishes the release only after the image succeeds.
