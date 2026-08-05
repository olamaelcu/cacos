# 2. PDS Migration Crate

Date: 2026-08-05

## Status

Accepted

## Context

cacos is a from-scratch Rust PDS. Storage splits across four SQLite databases (account, sequencer, did-cache, actor), each with its own schema, DDL, and migration history. The implementation needs typed column values and a centralized, versioned schema.

## Decision

The `migration` crate owns every sea-orm entity and four `MigratorTrait` implementations, one per database, each with its own bookkeeping table. `pds::db` re-exports the entities and opens+migrates each database with WAL journaling and foreign keys enabled, a busy timeout, and the `mode=rwc` URL form. Columns use typed values: `DbId` (ULID stored as BINARY(16)) for surrogate primary keys, `Did` for ATProto identifiers, and `time::OffsetDateTime` for timestamps. Functional and partial indexes are emitted as raw SQL.

## Consequences

Schema lives in one crate and each database self-migrates on open. Typed columns catch misuse at compile time at the cost of hand-written `ValueType` and `TryGetable` implementations. A small amount of index DDL is hand-maintained as raw SQL.
