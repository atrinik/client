#![forbid(unsafe_code)]
//! Synthetic deterministic fixtures for independent client tests.

use atrinik_actions::ActionRequest;
use atrinik_session::{ActionSink, RevisionedEvent, Session, SessionError, Snapshot};

#[derive(Default)]
pub struct RecordingSink {
    pub requests: Vec<ActionRequest>,
    pub reject: bool,
}
impl ActionSink for RecordingSink {
    fn send(&mut self, request: &ActionRequest) -> Result<(), SessionError> {
        if self.reject {
            Err(SessionError::TransportRejected)
        } else {
            self.requests.push(request.clone());
            Ok(())
        }
    }
}

pub fn replay(events: impl IntoIterator<Item = RevisionedEvent>) -> Result<Snapshot, SessionError> {
    let mut session = Session::default();
    for event in events {
        session.reduce(event)?;
    }
    Ok(session.snapshot())
}
