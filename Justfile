set shell := ["bash", "-eu", "-o", "pipefail", "-c"]
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
COVERAGE_FLOOR := "70"

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
    @echo "::group::strict-code"
    command -v rg >/dev/null || (echo "ripgrep is required for strict-code" && exit 1)
    ! rg -n '\b(TODO|FIXME)\b' --glob '*.rs' --glob '!target/**' . \
        | rg -v '\(#[0-9]+\)' \
        || (echo "bare TODO/FIXME — add (#NN) issue link" && exit 1)
    ! rg -n '#\[allow\([a-z_:]+\)\]' --glob '*.rs' --glob '!target/**' . \
        || (echo "#[allow(...)] missing reason = \"...\"" && exit 1)
    ! rg -n '\bunsafe[[:space:]]*\{' --glob '*.rs' --glob '!target/**' \
        --glob '!crates/windows-task/src/client/sys.rs' \
        --glob '!crates/windows-task/src/credentials.rs' \
        --glob '!crates/windows-task/src/handler.rs' . \
        || (echo "unsafe block outside an audited Windows boundary" && exit 1)
    ! rg -n '#!\[feature\(' --glob '*.rs' --glob '!target/**' . \
        || (echo "#![feature(...)] requires nightly — not allowed" && exit 1)
    @echo "::endgroup::"

lint: fmt-check clippy typos actionlint yamllint markdownlint strict-code

# ---------------------------------------------------------------------------
# Test, coverage, audit
# ---------------------------------------------------------------------------

test:
    cargo test --workspace --all-features

test-doc:
    cargo test --workspace --all-features --doc

docs:
    RUSTDOCFLAGS="-D warnings" cargo doc \
        -p windows-task -p windows-task-macros \
        --all-features --no-deps

coverage:
    mise exec github:taiki-e/cargo-llvm-cov -- cargo llvm-cov --workspace \
        --fail-under-regions {{COVERAGE_FLOOR}} \
        --summary-only

audit:
    mise exec github:EmbarkStudios/cargo-deny -- cargo-deny check

msrv:
    cargo +1.85.0 xtask msrv

windows-check:
    cargo xtask check-windows

package:
    cargo package -p windows-task-macros
    cargo package -p windows-task --list
    cargo package -p windows-task-cli --list

# ---------------------------------------------------------------------------
# Aggregate
# ---------------------------------------------------------------------------

ci: lint test test-doc docs audit windows-check
