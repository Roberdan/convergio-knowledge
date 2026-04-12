# ADR-002: Security Audit and Input Validation

**Date**: 2025-07-22
**Status**: Accepted
**Author**: Security Audit (Copilot)

## Context

A comprehensive security audit was performed on convergio-knowledge covering:
SQL injection, path traversal, command injection, SSRF, secret exposure,
race conditions, unsafe blocks, input validation, auth/authz bypass,
and vector DB query injection (LanceDB filter manipulation).

## Findings

### Critical — Fixed

1. **SQL injection in `seed.rs::has_table()`**: Table name was interpolated
   directly into a SQL string. Fixed with parameterized query (`?1`).

2. **LanceDB filter injection in `store.rs::delete()`**: The `id` parameter
   was only single-quote escaped. Added format validation (`ke-<uuid>` pattern)
   and forbidden character checks before building filter expressions.

3. **LanceDB filter injection in `store.rs::count_by_source()`**: Same pattern.
   Added `sanitize_filter_value()` with length and character validation.

4. **LanceDB filter injection in `store_pruning.rs::delete_batch()`**: Batch
   delete built an IN clause. Added per-ID validation before building filter.

### Medium — Fixed

5. **No input validation on write endpoint**: `WriteRequest` accepted any
   content length, arbitrary source_type/visibility strings. Added:
   - Content length limit (64 KiB)
   - Source type allowlist validation
   - Visibility enum validation ("org" | "public")
   - Source ID length limit (1-256 chars)

6. **No input validation on search endpoint**: `SearchRequest` had no bounds
   on `limit`, `min_score`, or `query` length. Added:
   - Limit clamped to 1-100
   - min_score clamped to 0.0-1.0
   - Query length limit (2000 chars)
   - Source type validation when provided

7. **No validation on prune endpoint**: `PruneRequest` accepted arbitrary
   max_age_days. Added bounds (1-3650) and source_type validation.

8. **Delete endpoint accepted arbitrary path IDs**: Added `ke-` prefix and
   length check before passing to store.

### Low — Accepted / Not Applicable

9. **No auth on routes**: Routes rely on the daemon's middleware layer for
   authentication and authorization (ring-based security model). Documented
   in README as an architectural assumption.

10. **Command execution in `hooks.rs::sync_recent_commits()`**: Uses
    `Command::new("git")` with hardcoded arguments only. Not injectable.

11. **`build.rs` uses `std::env::set_var`**: Runs single-threaded in build
    script context. Not a concern.

12. **Error messages may leak internals**: LanceDB error strings are returned
    in JSON. Acceptable for internal daemon API; not exposed to end users.

## Decision

- All critical and medium findings are fixed in this PR.
- Low-risk items are documented and accepted.
- Added 7 new unit tests covering validation logic.
- No unsafe blocks present in the crate.

## Consequences

- Write and search requests are now validated before processing.
- LanceDB filter expressions are protected against injection.
- SQLite queries use parameterized queries.
- Slightly stricter API contract (invalid inputs now rejected with errors).
