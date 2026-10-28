//! Job schedulers: the Redis side of create, edit, delete.
//!
//! A BullMQ job scheduler is: one member of the `repeat` sorted set (score =
//! next run), one hash `repeat:{id}` (name, `pattern` | `every`, `tz`, data
//! template, iteration count), and **one pending job** `repeat:{id}:{millis}`.
//! This module holds the scripts that write them and what a save returns;
//! validation and next runs live in `domain::schedule`.

/// `upsert_scheduler.lua` with the shared prelude.
pub fn upsert_scheduler_lua() -> String {
    super::maintenance::with_prelude(include_str!("lua/upsert_scheduler.lua"))
}

/// `remove_scheduler.lua` (remove, or pause = remove only the pending job).
pub fn remove_scheduler_lua() -> String {
    super::maintenance::with_prelude(include_str!("lua/remove_scheduler.lua"))
}
