//! emlbox — single-file .eml container: mmap reader, two-level index,
//! append-only delta log, KV client. See docs/FORMAT.md.

pub mod bench;
pub mod demo;
pub mod encoding;
pub mod format;
pub mod fs;
pub mod ipc;
pub mod kv;
pub mod mail;
pub mod pack;
pub mod query;
pub mod reader;
pub mod runner;
pub mod site;
pub mod rev;
pub mod sync;
pub mod tagdb;
pub mod verify;
pub mod writer;
