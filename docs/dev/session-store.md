# Session store

Where it lives: `rust/crates/acpira-host/src/store/transcript_store.rs` (the store), `store/db.rs` (connections, catalogue), `store/db/body.rs` (rows), `store/db/migration.rs` (compatibility files), `store/pack.rs` (cold packing of file-backed records).

- The session catalogue and migrated bodies live in `sessions.db` (rows per record head / turn head / block, `store/db/body.rs`); `index.json` is only the file backend's cache and the catalogue's first import. A session migrates on its first save: body transaction, then the pre-migration archive (only when a whole legacy file was there), then the compatibility stub.
- A catalogue reconcile never deletes a database body or changes where a body lives: a missing stub hides the session (`trashed_at`) until upkeep purges it weeks later, an unreadable one keeps its entry. Only a save, a revert, a copy from another machine or an explicit removal does.
- A save compares row hashes kept in memory (`SaveCache`), dropped when `PRAGMA data_version` shows another connection committed. A debounced save lets the session leave out subagents the store holds (`RecordSource::record_parts`); `flush` hands over everything.
- The stub trails the body (refreshed on a flush, on new blobs, on a foreign stamp, else after `STUB_REFRESH`), so the body is the record: a stub adds to it only what an older engine appended, and a whole record file newer than the body takes over from it.
- WAL or schema-open failure keeps the file backend, which refuses to write a session it could only load from its archive. Upkeep quarantines the database only when the check finds damage, never on a lock or I/O error. `ACPIRA_STORE=files` still reads the database and moves a body back to its file on save; `acpira store revert ID` does one at once.
- `sync_index` waits for chained record writes, else a fresh session's first write is mid-rename and another window takes it for deleted.
