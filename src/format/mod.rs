//! Declarative schemas for Valheim's save formats.
//!
//! Every on-disk structure is a `binrw` struct whose version-dependent fields
//! are gated with `#[br(if(version >= versions::...))]` and typed as `Option`.
//! Reading an older file leaves later fields `None`; `upgrade()` on the top
//! level type fills them with the game's defaults so the struct can be written
//! back in the current version, which is what the game itself does on save.
//!
//! A world comes in two layouts. Up to world version 39 it is a `.fwl` next
//! to a `.db` ([`fwl`], [`db`]); from 40 (game 1.0) it is a directory of
//! `_main.<N>.*` files and per-region `.chunk` files ([`chunked`]).
//! [`world::WorldData`] holds either and is what commands work on.

pub mod chunked;
pub mod compress;
pub mod db;
pub mod fch;
pub mod fwl;
pub mod inventory;
pub mod migrate;
pub mod primitives;
pub mod versions;
pub mod world;
pub mod zdo;
