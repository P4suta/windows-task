set shell := ["bash", "-eu", "-o", "pipefail", "-c"]
set windows-shell := ["pwsh", "-NoLogo", "-NoProfile", "-Command"]
set dotenv-load := false

# Tooling is provisioned by mise (mise.toml + .config/mise/config.toml);
# run `just bootstrap` once after cloning. The rust toolchain is pinned
# in rust-toolchain.toml. cargo plugins (nextest / deny / llvm-cov) are
# invoked through an explicit `mise exec <tool>` where needed so hooks — which
# spawn under /bin/sh with a PATH that can lag a fresh install — resolve them
# deterministically without installing unrelated development tools.
#
# This is a host/mise workflow: plain `cargo`, no container required.
# Repos that need a reproducible OS envelope (locale / Unicode
# sensitivity) add the optional `docker-dev` layer on top.

# Portable code is covered on Linux; native COM/Event Log paths are exercised
# by the Windows smoke job. Keep coverage informative until the two reports can
# be merged without pretending the platform-specific half is untested.

default:
    @just --list --unsorted

# ---------------------------------------------------------------------------
# Bootstrap
# ---------------------------------------------------------------------------

bootstrap:
    # Cargo-backed tools share rustup state, so install sequentially.
    mise install --jobs=1
    just hooks

hooks:
    lefthook install

hooks-uninstall:
    lefthook uninstall

# ---------------------------------------------------------------------------
# Build / run
# ---------------------------------------------------------------------------

build:
    cargo build --workspace --all-targets --all-features

watch JOB="check":
    bacon {{JOB}}

run +ARGS:
    cargo run -- {{ARGS}}

# ---------------------------------------------------------------------------
# Lint / format
# ---------------------------------------------------------------------------

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

typos:
    typos

actionlint:
    actionlint

yamllint:
    yamllint .

markdownlint:
    markdownlint-cli2 "**/*.md" "#target"

# Reject patterns that mask real bugs even when type / lint gates pass.
strict-code:
    cargo xtask strict-code

lint: fmt-check clippy typos actionlint yamllint markdownlint strict-code

# ---------------------------------------------------------------------------
# Test, coverage, audit
# ---------------------------------------------------------------------------

test:
    cargo xtask test --suite portable

test-doc:
    cargo test --workspace --all-features --doc

docs:
    RUSTDOCFLAGS="-D warnings" cargo doc \
        -p windows-task -p windows-task-macros \
        --all-features --no-deps

coverage:
    cargo xtask coverage

audit:
    mise exec github:EmbarkStudios/cargo-deny -- cargo-deny check

msrv:
    cargo +1.85.0 xtask msrv

windows-check:
    cargo xtask check-windows

package:
    cargo xtask package

# ---------------------------------------------------------------------------
# Aggregate
# ---------------------------------------------------------------------------

ci:
    cargo xtask ci
