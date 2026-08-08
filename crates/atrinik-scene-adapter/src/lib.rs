#![forbid(unsafe_code)]
//! Client-owned immutable bridge to the future released renderer scene contract.

use atrinik_session::Snapshot;

pub const RENDERER_CONTRACT: &str = "scene-snapshot-1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameEntity {
    pub stable_id: u64,
    pub generation: u32,
    pub x: i32,
    pub y: i32,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameInput {
    pub session_revision: u64,
    pub map_generation: u64,
    pub entities: Vec<FrameEntity>,
}

pub fn frame(snapshot: &Snapshot) -> FrameInput {
    FrameInput {
        session_revision: snapshot.revision,
        map_generation: snapshot.map_generation,
        entities: snapshot
            .entities
            .iter()
            .map(|entity| FrameEntity {
                stable_id: entity.handle.object_id,
                generation: entity.handle.object_generation,
                x: entity.x,
                y: entity.y,
            })
            .collect(),
    }
}
