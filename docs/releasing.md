# Releasing

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

Finally, create and push `v<workspace-version>`. The release workflow verifies
the tag, builds x64 and ARM64 CLI archives on Windows, adds SHA-256 checksum
files, and creates the GitHub release. Rerunning it updates the same assets
rather than creating another release.
