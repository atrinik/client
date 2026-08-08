#![forbid(unsafe_code)]
//! Boundary where future released Game Protocol 1 messages are validated.

use atrinik_actions::ObjectHandle;
use atrinik_session::{Entity, Event, RevisionedEvent, SessionError};

pub const PROTOCOL_CONTRACT: &str = "game-protocol-1";
pub const MAX_WIRE_BYTES: usize = 1 << 20;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidatedMessage {
    Connected,
    EnteredWorld,
    MapReset {
        map_generation: u64,
    },
    Entity {
        object_id: u64,
        object_generation: u32,
        map_generation: u64,
        x: i32,
        y: i32,
        name: String,
    },
    Message(String),
    Disconnected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Envelope {
    pub revision: u64,
    pub session_generation: u64,
    pub payload_bytes: usize,
    pub message: ValidatedMessage,
}

pub fn into_domain(envelope: Envelope) -> Result<RevisionedEvent, SessionError> {
    if envelope.payload_bytes > MAX_WIRE_BYTES
        || envelope.revision == 0
        || envelope.session_generation == 0
    {
        return Err(SessionError::InvalidValue);
    }
    let event = match envelope.message {
        ValidatedMessage::Connected => Event::Connected,
        ValidatedMessage::EnteredWorld => Event::EnteredWorld,
        ValidatedMessage::MapReset { map_generation } => Event::MapReset { map_generation },
        ValidatedMessage::Entity {
            object_id,
            object_generation,
            map_generation,
            x,
            y,
            name,
        } => Event::EntityUpsert(Entity {
            handle: ObjectHandle {
                session_generation: envelope.session_generation,
                map_generation,
                object_id,
                object_generation,
            },
            x,
            y,
            name,
        }),
        ValidatedMessage::Message(value) => Event::Message(value),
        ValidatedMessage::Disconnected => Event::Disconnected,
    };
    Ok(RevisionedEvent {
        revision: envelope.revision,
        session_generation: envelope.session_generation,
        event,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_envelopes_never_reach_session() {
        let envelope = Envelope {
            revision: 1,
            session_generation: 1,
            payload_bytes: MAX_WIRE_BYTES + 1,
            message: ValidatedMessage::Connected,
        };
        assert_eq!(into_domain(envelope), Err(SessionError::InvalidValue));
    }
}
