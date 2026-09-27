//! IPC commands. Every command maps engine errors to `{ code, message }`
//! (see `nzap_core::ErrorPayload`); streaming commands take a `Channel`
//! and an optional `streamId` for `stream_cancel`.

pub mod app;
pub mod auth;
pub mod dialogs;
pub mod files;
pub mod notebooks;
pub mod sessions;
