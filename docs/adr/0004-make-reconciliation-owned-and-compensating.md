# 4. Make reconciliation owned and compensating

- Status: accepted
- Date: 2026-09-02
- Deciders: project maintainers
- Tags: desired-state, safety, credentials

## Context

Task Scheduler has no transaction spanning folders, registrations, enabled
state, and ACLs. Blind synchronization can overwrite administrator-created
tasks, fire registration triggers, or delete unrelated tasks. Password-backed
principal secrets cannot be read back for rollback.

## Decision

Give each manifest a stable owner UUID and namespace. Encode a deterministic
owner/task marker in `RegistrationInfo.URI`. Refuse unowned collisions unless
adoption is explicit, and prune only matching owner markers when pruning is
explicit.

Before mutation, gather the live state, construct a deterministic plan, resolve
all desired credentials, and capture raw XML plus owner/group/DACL. Classify
password-backed rollback as requiring a separate credential. Reject incomplete
rollback preparation unless the caller explicitly allows irreversible work.

Suppress registration triggers by default. On the first failure, compensate
completed changes in reverse order and return both the initiating failure and
any compensation failures.

## Consequences

- Default apply cannot overwrite or prune unrelated tasks.
- A plan is reviewable and stable enough for automation.
- No plaintext credential is needed in a manifest; Credential Manager targets
  or application resolvers remain non-secret references.
- Compensation is not a true transaction: service loss or an explicitly
  irreversible change can leave uncertain state.
- Changing an old password-backed identity may require separate desired and
  rollback credentials.

## Alternatives considered

- **Use task path prefixes as ownership.** Rejected because paths can predate the
  tool or be shared with administrators.
- **Store passwords in manifests.** Rejected because source-controlled desired
  state is not a secret store.
- **Best-effort rollback without preflight.** Rejected because discovering a
  missing password after partial mutation is too late.
