# Releasing

`just audit` checks the unchanged dependency policy against a freshly cloned advisory database and keeps its configuration and logs under `target/verification`.

Release preparation is intentionally separate from publication. Update the
workspace version and changelog, then run:

```sh
cargo +1.85.0 xtask ci
just lint
cargo +1.85.0 package -p windows-task-macros
cargo +1.85.0 package -p windows-task --list
cargo +1.85.0 package -p windows-task-cli --list
```

For the first release of a version, crates.io dependencies must become visible
in dependency order:

```sh
cargo +1.85.0 publish -p windows-task-macros
cargo +1.85.0 publish -p windows-task
cargo +1.85.0 publish -p windows-task-cli
```

Review each package before running these commands. Publication is deliberately
not part of CI and needs an authenticated maintainer. The three archives each
include the shared README, Apache-2.0 and MIT texts, and NOTICE.

Finally, create and push `v<workspace-version>`.
The release workflow verifies the tag, builds x64 and ARM64 CLI archives on Windows, and adds SHA-256 checksum files.
It attaches the complete artifact set to an unpublished draft before final publication.
A retry reuses draft assets only when their size and SHA-256 match, uploads missing assets, and rejects conflicting or unexpected assets before publication.
Legacy assets without a recorded digest are verified from downloaded bytes.
Final publication verifies that the version tag exists remotely.
Published releases and their assets cannot be updated; corrections require a new version.
