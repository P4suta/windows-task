# CLI guide

The `windows-task` binary prints structured JSON for finite commands and
newline-delimited JSON for `history watch`. Exit code `0` means success, `1`
means an operation error, and `2` means validation findings or a non-empty
dry-run plan.

## Connections and credentials

Commands connect locally with the current token unless `--computer` is set.
For remote authentication, store a Generic Credential in Windows Credential
Manager and pass only its target name:

```powershell
cmdkey /generic:windows-task/prod /user:CONTOSO\automation /pass
windows-task --computer server01 --connection-credential windows-task/prod doctor
```

Connection flags are global and may appear before or after a subcommand. A
manifest can also declare one common `credentials.connection` reference across
its tasks. Conflicting references are rejected.

Registration passwords work the same way. Raw password-backed XML needs
`task register --registration-credential TARGET`. Manifest tasks use
`credentials.registration`; apply can override it with
`--registration-credential`. Use `--rollback-credential` when restoring the
old task requires a different secret.

Secrets are copied into zeroizing UTF-16 buffers, never printed through
`Debug`, and never accepted as manifest values or CLI password arguments.

## Inspection

```powershell
windows-task capabilities
windows-task doctor
windows-task task get '\Vendor\Task'
windows-task task get '\Vendor\Task' --raw
windows-task task list '\Vendor' --recursive --hidden
windows-task task running --hidden
windows-task folder list '\Vendor' --recursive
windows-task folder security '\Vendor'
```

`task get --raw` is the lossless export path. JSON output includes the typed
definition when its schema is understood; the library API always retains the
exact source bytes even if typed decoding fails.

## Mutation

```powershell
windows-task task register '\Acme\Backup' .\backup.xml --yes
windows-task task disable '\Acme\Backup'
windows-task task enable '\Acme\Backup'
windows-task task run '\Acme\Backup' --parameter nightly --wait
windows-task task stop --path '\Acme\Backup'
windows-task task stop --instance 550e8400-e29b-41d4-a716-446655440000
windows-task task delete '\Acme\Backup' --yes
```

Registration triggers are suppressed unless
`task register --allow-registration-triggers` is given. Delete does not stop
running instances implicitly. Registration, folder/task delete, desired-state
apply, Event Log configuration, and ACL replacement require `--yes`. Use
`validate --native` for non-persisting target validation.

## History

```powershell
windows-task history query --task '\Acme\Backup' --since-seconds 3600 --limit 100
windows-task history watch --task '\Acme\Backup' --poll-milliseconds 250
windows-task history enable --yes
```

Querying never changes channel configuration. Local history uses the current
token; remote history opens a separate Event Log RPC session with the same
one-shot connection credential. `task run --wait` requires completion matched by
instance GUID. Inaccessible history is an error. Add `--allow-polling-fallback`
to explicitly permit the registered task's last result as an estimate; the
result includes `polling_fallback` confidence and a reason. Concurrent runs can
change that estimate. Watch uses bounded forward pages and stops with
`HistoryGap` when a bookmark is no longer available. `QueryLimit` means a query
did not establish a complete result within its scan/return bounds.

## Desired state

```powershell
windows-task validate .\tasks.toml
windows-task validate .\tasks.toml --native
windows-task plan .\tasks.toml --prune
windows-task apply .\tasks.toml --prune --yes
```

`validate` is portable unless `--native` is requested. `plan` is always live
and read-only. `apply` first repeats inspection and planning, resolves every
desired and rollback credential, and captures raw XML plus owner/group/DACL
before its first mutation.

Safety switches are intentionally independent:

- `--adopt` permits replacing an unowned collision.
- `--prune` permits deleting tasks with this manifest's ownership marker only.
- `--stop-running` stops instances before task update/delete.
- `--allow-irreversible` proceeds when an exact rollback password or ACL cannot
  be captured.

On failure, applied steps are compensated in reverse order. The library's
`ApplyFailure` retains both the initiating error and any rollback failures.
CLI apply prints a JSON report on either outcome and retains exit code 1 on
failure. Inspect `cause`, `report.journal`, `report.unresolved`,
`report.rollback_failures`, and `report.irreversible_effects`. Restored definitions
do not undo stopped instances. External changes block automatic compensation.

Keep the original failure before inspecting the remaining state:

```powershell
windows-task apply .\tasks.toml --yes > .\apply-result.json 2> .\apply.stderr.log
$applyExit = $LASTEXITCODE
if ($applyExit -ne 0) {
    windows-task plan .\tasks.toml > .\remaining-plan.json 2> .\inspection.stderr.log
    $incident = '.\incident-' + [guid]::NewGuid()
    windows-task doctor --bundle $incident > .\doctor-result.json
}
```

`plan` and `doctor` perform read-only checks. If inspection also fails, preserve
its error alongside the original apply report; an empty inspection file is not
evidence of successful recovery. Resolve `unresolved` targets and credential or
ownership conflicts before issuing another apply. A new run does not replace the
original failure journal.

## Diagnostic bundles and tracing

```powershell
windows-task --log-level debug --log-format json --log-file .\operations.jsonl doctor --bundle .\incident-001
```

`doctor` includes connection failures and performs only read operations. The
bundle directory must be new. It contains platform information, redacted
connection settings, diagnostic checks (including failures), and reproduction
arguments. Target names, credential references, XML and error message bodies are
excluded from the bundle. Write/registration rights remain unverified. The CLI
owns tracing setup; library consumers configure their own subscriber.

## Handler registration

Handler DLL registry automation lives under `cargo xtask`, keeping it out of
normal scheduler commands:

```powershell
cargo xtask handler reg-file --clsid CLSID --dll C:\full\handler.dll `
  --scope user --output handler.reg
cargo xtask handler register --clsid CLSID --dll C:\full\handler.dll `
  --scope user --yes
cargo xtask handler unregister --clsid CLSID --scope user --yes
```

Per-machine registration usually requires elevation. The generated
`ThreadingModel` is `Both`; each DLL must match the target host architecture.
