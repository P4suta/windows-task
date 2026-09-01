# Contributing to windows-task

Thank you for considering a contribution. Notes on the parts that are
easy to get wrong on a first pass:

## Development environment

Rust 1.85 is the minimum supported version. `mise` installs the pinned
toolchain and the repository's lint/audit utilities:

```sh
just bootstrap    # mise install + lefthook install
just lint         # all configured lint passes
just test         # workspace tests with every feature
just windows-check # compile x64 and ARM64 Windows targets
just ci           # the complete local CI surface
```

`just hooks` installs the lefthook pre-commit / commit-msg / pre-push
hooks; anything that would fail in CI will fail locally first.

Portable tests run on any host. On Windows, read-only Task Scheduler smoke
tests run normally. Set `WINDOWS_TASK_MUTATION_TESTS=1` only on a disposable CI
host to exercise creation and cleanup of an isolated, disabled SYSTEM task.

Unsafe code is confined to `client/sys.rs`, `credentials.rs`, and the native
portion of `handler.rs`. Changes there should explain every new unsafe
operation and be checked on both supported Windows targets.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/). The
commit-msg hook enforces the type prefix; scope is optional. Allowed
types: `feat` / `fix` / `docs` / `style` / `refactor` / `perf` /
`test` / `build` / `ci` / `chore` / `revert`.

## Pull requests

- Branch off `main`, push, open a PR.
- Squash-merge by default. The PR title becomes the squashed commit
  message — write it as a Conventional Commit subject.
- CI must be green; reviewer approval required.

## License

By contributing you agree that your contribution is dual-licensed under
Apache-2.0 OR MIT, the same terms as the project itself.
