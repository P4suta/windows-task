# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/).

## [Unreleased]

### Added

- Complete owned Task Scheduler 2.0 model for schema versions 1.2 through 1.6.
- Bounded, lossless UTF-8/UTF-16 Task XML parsing and canonical writing.
- Dedicated-MTA local and remote scheduler clients with blocking and
  runtime-neutral async APIs.
- Typed Operational Event Log history, watchers, and run-instance correlation.
- Ownership-safe manifest planning, apply, credential preflight, and
  reverse-order compensation.
- Schedule recipes, an exact five-field cron compiler, and stable diagnostics.
- Official CLI and safe COM handler runtime/proc macro with registry tooling.
