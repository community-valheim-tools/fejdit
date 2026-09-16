//! World metadata (`World.SaveWorldFWLData` / `World.LoadWorld`): the `.fwl`
//! beside a `.db`, or the `_main.<N>.fwl2` inside a chunked world directory.
//! Same payload either way, and version 41 appends the player history.
//!
//! On disk: `i32` payload length, then the payload below.

use anyhow::{Context, Result, bail};
use binrw::io::Cursor;
use binrw::{BinRead, BinWrite, binrw};
use serde::Serialize;

use super::primitives::{Bool, CsString, StringList};
use super::versions::{WORLD_GEN_LEGACY, world};

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorldMeta {
    pub version: i32,
    pub name: CsString,
    pub seed_name: CsString,
    pub seed: i32,
    pub uid: i64,
    #[br(if(version >= world::WORLD_GEN_VERSION))]
    pub world_gen_version: Option<i32>,
    #[br(if(version >= world::NEEDS_DB))]
    pub needs_db: Option<Bool>,
    #[br(if(version >= world::STARTING_GLOBAL_KEYS))]
    pub starting_global_keys: Option<StringList>,
    /// Everyone who has joined the world (`ZNet.m_playerHistory`).
    #[br(if(version >= world::PLAYER_HISTORY))]
    pub player_history: Option<PlayerHistory>,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PlayerHistory {
    #[br(temp)]
    #[bw(calc = players.len() as i32)]
    count: i32,
    #[br(count = count)]
    pub players: Vec<PlayerHistoryEntry>,
}

/// `ZNet.CrossNetworkUserInfo.Write`.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PlayerHistoryEntry {
    /// `PlatformUserID` as text, e.g. `Steam_76561198000000000`.
    pub id: CsString,
    pub display_name: CsString,
    pub server_assigned_display_name: CsString,
    pub playfab_id: CsString,
}

impl WorldMeta {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let len = i32::read_le(&mut cursor).context("reading .fwl payload length")?;
        let len = usize::try_from(len).context("negative .fwl payload length")?;
        let start = 4;
        let end = start + len;
        if end > bytes.len() {
            bail!(
                ".fwl payload length {len} exceeds file size {}",
                bytes.len()
            );
        }
        let mut payload = Cursor::new(&bytes[start..end]);
        let meta = WorldMeta::read_le(&mut payload).context("parsing .fwl payload")?;
        let consumed = payload.position() as usize;
        if consumed != len {
            bail!(
                ".fwl payload has {} unparsed trailing bytes",
                len - consumed
            );
        }
        Ok(meta)
    }

    pub fn to_file_bytes(&self) -> Result<Vec<u8>> {
        let mut payload = Cursor::new(Vec::new());
        self.write_le(&mut payload)
            .context("serializing .fwl payload")?;
        let payload = payload.into_inner();
        let mut out = Cursor::new(Vec::with_capacity(payload.len() + 4));
        (payload.len() as i32).write_le(&mut out)?;
        std::io::Write::write_all(&mut out, &payload)?;
        Ok(out.into_inner())
    }

    /// Fill fields absent from older versions with the game's defaults and
    /// stamp the current version, mirroring what the game does when it
    /// saves. The current version implies the chunked layout, so a `.fwl`
    /// upgraded this way belongs in a world directory as a `.fwl2`; see
    /// [`super::world::WorldData::upgrade`].
    pub fn upgrade(&mut self) {
        self.version = world::CURRENT;
        self.world_gen_version.get_or_insert(WORLD_GEN_LEGACY);
        self.needs_db.get_or_insert(Bool(true));
        self.starting_global_keys
            .get_or_insert_with(StringList::default);
        self.player_history
            .get_or_insert_with(PlayerHistory::default);
    }

    /// Whether this metadata describes a chunked world directory rather
    /// than a `.fwl` + `.db` pair (`World.LoadWorld` refuses the other
    /// combination).
    pub fn is_chunked(&self) -> bool {
        self.version >= world::CHUNKED_SAVE
    }
}
