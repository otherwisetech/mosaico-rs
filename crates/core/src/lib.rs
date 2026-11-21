//! mosaico core: everything except the window. Scans source folders for photos and videos,
//! works out each one's capture date, makes thumbnails, builds an organization plan, applies it
//! (copy by default, move on request) and undoes it from a journal.
//!
//! Safety rules live here, not in the UI: plans are read-only, nothing is ever overwritten,
//! copies are verified, the only file ever removed is a move's verified source, and every write
//! is journaled before it happens.
pub mod apply;
mod bmff;
pub mod date;
pub mod dirs;
pub mod fixtures;
pub mod hash;
pub mod journal;
pub mod media;
pub mod meta;
pub mod people;
pub mod plan;
pub mod scan;
pub mod thumb;
pub mod timeline;
mod tiff;

pub use date::DateSource;
pub use media::{Format, Kind};
pub use plan::{Mode, Plan};
pub use scan::{Item, Library};

#[cfg(test)]
mod fixture_tests;
