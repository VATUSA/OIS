//! `ois-core` — shared, pure domain types for the OIS platform.
//!
//! This crate is the ported, database-free heart of osmium's authorization model:
//! path-based permissions (`segments.action`), the permission-tree JSON used by the
//! access editor UI, and the OIS permission/role catalog. Anything that needs a
//! Postgres pool (effective-permission resolution, grants) lives in the `backend`
//! crate; everything here is pure and reusable by the Discord bot.

pub mod catalog;
pub mod permissions;

pub use permissions::{PermissionAction, PermissionPath};
