# Branch and release policy

This document is the source of truth for branches, version numbers and published builds. GitHub Actions currently validates and builds the project; publishing a GitHub Release is a separate, deliberate step.

## Branches

- `master` is the stable line. Only changes considered ready for a stable release go here.
- `alpha` is the integration and preview line. It may contain changes that are still being evaluated.
- Work on short lived branches such as `feat/<description>` or `fix/<description>`, then open a pull request into `alpha`.
- Promote a tested state from `alpha` to `master` with a pull request. Fixes made directly on `master` should also be merged back to `alpha`.
- Pull requests and pushes to `alpha` and `master` must pass formatting, workspace tests and the Windows, Linux and macOS build matrix before merge. Configure GitHub branch rules to require these checks; workflow files alone do not enforce merge protection.

## Versions

Use Semantic Versioning: `MAJOR.MINOR.PATCH` for stable releases and SemVer prerelease identifiers for previews.

- Alpha versions use `MAJOR.MINOR.PATCH-alpha.N`, for example `0.1.0-alpha.2`.
- Stable versions use `MAJOR.MINOR.PATCH`, for example `0.1.0`.
- Keep the workspace version in `Cargo.toml` and the resolved package versions in `Cargo.lock` aligned with the release. Update the displayed version in the README and `site/index.html` as part of the same change.
- Each version is released once. Do not move or reuse a published version tag; increment the prerelease number or version instead.

## Alpha releases

1. Merge the candidate changes into `alpha` and wait for CI and all platform builds to pass.
2. Update the version to the next alpha number and commit that change to `alpha`.
3. Create and push the matching tag, such as `v0.1.0-alpha.2`, on that commit.
4. Publish it as a **pre-release** on GitHub after checks pass. Attach the four platform ZIPs produced by the build workflow.

Alpha tags are for evaluation. Their configuration, protocol and behavior may change before a stable release. Do not publish them to crates.io or another package registry under this policy.

## Stable releases

1. Promote the release candidate to `master` through a pull request after CI and all platform builds pass.
2. Set the workspace and README versions to the stable SemVer version and merge the change.
3. Create and push the matching tag, such as `v0.1.0`, on the approved `master` commit.
4. After checks pass, create a normal GitHub Release with the four platform ZIPs. Require a maintainer's explicit approval before publication.

## What Actions currently does

`CI` runs formatting and workspace tests. `Build` compiles the full distribution for Windows x86_64, Linux x86_64, macOS ARM64 and macOS Intel, then stores a ZIP for each as a workflow artifact. These artifacts are available from the Actions run; they are not yet attached to a GitHub Release automatically. The release steps above describe the publication policy and must be wired into Actions before publication can be automated.
