//! Data model for the UI. The actual types live in the shared `wisp-dto`
//! crate (`crates/wisp-dto`), which owns the serde contract between the UI and
//! the Tauri backend; this module re-exports them under the historical
//! `crate::dto::*` paths. Backend contract tests in `src-tauri` deserialize
//! command output into these same types to catch drift.

pub(crate) use wisp_dto::*;
