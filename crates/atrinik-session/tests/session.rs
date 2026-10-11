use atrinik_actions::{Action, ActionRequest, Direction, ObjectHandle};
use atrinik_session::{
    ActionSink, Entity, Event, Item, Phase, RevisionedEvent, Session, SessionError,
};

#[derive(Default)]
struct Sink {
    requests: Vec<ActionRequest>,
    fail: bool,
}
impl ActionSink for Sink {
    fn send(&mut self, request: &ActionRequest) -> Result<(), SessionError> {
        if self.fail {
            return Err(SessionError::TransportRejected);
        }
        self.requests.push(request.clone());
        Ok(())
    }
}

fn reduce(session: &mut Session, revision: u64, generation: u64, event: Event) {
    session
        .reduce(RevisionedEvent {
            revision,
            session_generation: generation,
            event,
        })
        .expect("event accepted");
}

fn playing_session() -> (Session, ObjectHandle) {
    let mut session = Session::default();
    reduce(&mut session, 1, 1, Event::Connected);
    reduce(&mut session, 2, 1, Event::EnteredWorld);
    reduce(&mut session, 3, 1, Event::MapReset { map_generation: 1 });
    let handle = ObjectHandle {
        session_generation: 1,
        map_generation: 1,
        object_id: 9,
        object_generation: 1,
    };
    reduce(
        &mut session,
        4,
        1,
        Event::EntityUpsert(Entity {
            handle,
            x: 2,
            y: 4,
            name: "target".into(),
        }),
    );
    (session, handle)
}

#[test]
fn revisions_and_mutations_fail_atomically() {
    let (mut session, _) = playing_session();
    let before = session.snapshot();
    assert_eq!(
        session.reduce(RevisionedEvent {
            revision: 6,
            session_generation: 1,
            event: Event::Message("gap".into())
        }),
        Err(SessionError::RevisionGap)
    );
    assert_eq!(session.snapshot(), before);
    assert_eq!(
        session.reduce(RevisionedEvent {
            revision: 5,
            session_generation: 1,
            event: Event::PlayerStats {
                health: 2,
                health_max: 1
            }
        }),
        Err(SessionError::InvalidValue)
    );
    assert_eq!(session.snapshot(), before);
}

#[test]
fn stale_handles_and_map_reset_are_rejected() {
    let (mut session, handle) = playing_session();
    reduce(&mut session, 5, 1, Event::MapReset { map_generation: 2 });
    let before = session.snapshot();
    assert_eq!(
        session.reduce(RevisionedEvent {
            revision: 6,
            session_generation: 1,
            event: Event::EntityRemoved(handle)
        }),
        Err(SessionError::StaleHandle)
    );
    assert_eq!(session.snapshot(), before);
    let mut sink = Sink::default();
    assert_eq!(
        session.dispatch(
            ActionRequest {
                id: 1,
                action: Action::Attack(handle)
            },
            &mut sink
        ),
        Err(SessionError::StaleHandle)
    );
}

#[test]
fn snapshots_reconstruct_state_and_actions_are_only_local_intent() {
    let (mut session, _) = playing_session();
    reduce(
        &mut session,
        5,
        1,
        Event::PlayerStats {
            health: 8,
            health_max: 10,
        },
    );
    let item_handle = ObjectHandle {
        session_generation: 1,
        map_generation: 0,
        object_id: 99,
        object_generation: 1,
    };
    reduce(
        &mut session,
        6,
        1,
        Event::InventoryReplay(vec![Item {
            handle: item_handle,
            quantity: 2,
            name: "fixture".into(),
        }]),
    );
    reduce(&mut session, 7, 1, Event::DialogReplaced("dialog".into()));
    reduce(&mut session, 8, 1, Event::QuestReplaced("quest".into()));
    reduce(&mut session, 9, 1, Event::Message("message".into()));
    let mut sink = Sink::default();
    session
        .dispatch(
            ActionRequest {
                id: 7,
                action: Action::Move(Direction::North),
            },
            &mut sink,
        )
        .expect("local acceptance");
    let snapshot = session.snapshot();
    assert_eq!(snapshot.phase, Phase::Playing);
    assert_eq!(snapshot.player.health, 8);
    assert_eq!(snapshot.inventory.len(), 1);
    assert_eq!(snapshot.dialog, "dialog");
    assert_eq!(snapshot.quest, "quest");
    assert_eq!(snapshot.messages, ["message"]);
    assert_eq!(snapshot.pending_action_ids, [7]);
    assert_eq!(sink.requests.len(), 1);
}

#[test]
fn disconnect_clears_all_transient_intent_and_state() {
    let (mut session, _) = playing_session();
    let mut sink = Sink::default();
    session
        .dispatch(
            ActionRequest {
                id: 1,
                action: Action::Move(Direction::East),
            },
            &mut sink,
        )
        .expect("move accepted");
    reduce(&mut session, 5, 1, Event::Disconnected);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.phase, Phase::Disconnected);
    assert!(
        snapshot.entities.is_empty()
            && snapshot.inventory.is_empty()
            && snapshot.pending_action_ids.is_empty()
    );
    assert_eq!(snapshot.player.movement, None);
}

#[test]
fn transport_failure_does_not_create_pending_intent() {
    let (mut session, _) = playing_session();
    let before = session.snapshot();
    let mut sink = Sink {
        fail: true,
        ..Sink::default()
    };
    assert_eq!(
        session.dispatch(
            ActionRequest {
                id: 1,
                action: Action::Stop
            },
            &mut sink
        ),
        Err(SessionError::TransportRejected)
    );
    assert_eq!(session.snapshot(), before);
}

#[test]
fn text_and_replay_inputs_are_bounded() {
    let (mut session, _) = playing_session();
    assert_eq!(
        session.reduce(RevisionedEvent {
            revision: 5,
            session_generation: 1,
            event: Event::Message("x".repeat(513))
        }),
        Err(SessionError::TextLimit)
    );
    let item_handle = ObjectHandle {
        session_generation: 1,
        map_generation: 0,
        object_id: 99,
        object_generation: 1,
    };
    let duplicate = Item {
        handle: item_handle,
        quantity: 1,
        name: "item".into(),
    };
    assert_eq!(
        session.reduce(RevisionedEvent {
            revision: 5,
            session_generation: 1,
            event: Event::InventoryReplay(vec![duplicate.clone(), duplicate])
        }),
        Err(SessionError::InvalidValue)
    );
}

#[test]
fn retired_entity_and_item_generations_cannot_be_reused() {
    let (mut session, handle) = playing_session();
    reduce(&mut session, 5, 1, Event::EntityRemoved(handle));
    let stale = Entity {
        handle,
        x: 0,
        y: 0,
        name: "stale".into(),
    };
    assert_eq!(
        session.reduce(RevisionedEvent {
            revision: 6,
            session_generation: 1,
            event: Event::EntityUpsert(stale)
        }),
        Err(SessionError::StaleHandle)
    );
    let next = ObjectHandle {
        object_generation: 2,
        ..handle
    };
    reduce(
        &mut session,
        6,
        1,
        Event::EntityUpsert(Entity {
            handle: next,
            x: 0,
            y: 0,
            name: "next".into(),
        }),
    );

    let item = ObjectHandle {
        session_generation: 1,
        map_generation: 0,
        object_id: 22,
        object_generation: 4,
    };
    reduce(
        &mut session,
        7,
        1,
        Event::InventoryReplay(vec![Item {
            handle: item,
            quantity: 1,
            name: "item".into(),
        }]),
    );
    reduce(&mut session, 8, 1, Event::InventoryReplay(vec![]));
    assert_eq!(
        session.reduce(RevisionedEvent {
            revision: 9,
            session_generation: 1,
            event: Event::InventoryReplay(vec![Item {
                handle: item,
                quantity: 1,
                name: "stale".into()
            }])
        }),
        Err(SessionError::StaleHandle)
    );
}

#[test]
fn character_selection_is_valid_only_before_entering_world() {
    let mut session = Session::default();
    reduce(&mut session, 1, 1, Event::Connected);
    let mut sink = Sink::default();
    session
        .dispatch(
            ActionRequest {
                id: 1,
                action: Action::SelectCharacter { character_id: 8 },
            },
            &mut sink,
        )
        .expect("character selection");
    reduce(&mut session, 2, 1, Event::EnteredWorld);
    assert_eq!(
        session.dispatch(
            ActionRequest {
                id: 2,
                action: Action::SelectCharacter { character_id: 8 }
            },
            &mut sink
        ),
        Err(SessionError::InvalidTransition)
    );
}

#[test]
fn protected_connection_requires_access_acceptance_before_account_flow() {
    let mut session = Session::default();
    reduce(&mut session, 1, 1, Event::AccessRequired);
    assert_eq!(session.snapshot().phase, Phase::AccessRequired);

    let mut sink = Sink::default();
    assert_eq!(
        session.dispatch(
            ActionRequest {
                id: 1,
                action: Action::SelectCharacter { character_id: 8 },
            },
            &mut sink,
        ),
        Err(SessionError::InvalidTransition)
    );

    reduce(&mut session, 2, 1, Event::AccessAuthenticationSent);
    assert_eq!(session.snapshot().phase, Phase::AccessPending);
    reduce(&mut session, 3, 1, Event::AccessAccepted);
    assert_eq!(session.snapshot().phase, Phase::Connected);
    session
        .dispatch(
            ActionRequest {
                id: 1,
                action: Action::SelectCharacter { character_id: 8 },
            },
            &mut sink,
        )
        .expect("account and character flow follows access acceptance");
}

#[test]
fn unavailable_access_closes_attempt_without_reusing_its_generation() {
    let mut session = Session::default();
    reduce(&mut session, 1, 1, Event::AccessRequired);
    reduce(&mut session, 2, 1, Event::AccessAuthenticationSent);
    reduce(&mut session, 3, 1, Event::AccessUnavailable);
    assert_eq!(session.snapshot().phase, Phase::Disconnected);

    assert_eq!(
        session.reduce(RevisionedEvent {
            revision: 4,
            session_generation: 1,
            event: Event::AccessAccepted,
        }),
        Err(SessionError::InvalidTransition)
    );
    reduce(&mut session, 4, 2, Event::Connected);
    assert_eq!(session.snapshot().phase, Phase::Connected);
}

#[test]
fn access_events_are_rejected_out_of_order_atomically() {
    let mut before_hello = Session::default();
    let initial = before_hello.snapshot();
    assert_eq!(
        before_hello.reduce(RevisionedEvent {
            revision: 1,
            session_generation: 0,
            event: Event::AccessAccepted,
        }),
        Err(SessionError::InvalidTransition)
    );
    assert_eq!(before_hello.snapshot(), initial);

    let mut protected = Session::default();
    reduce(&mut protected, 1, 1, Event::AccessRequired);
    let required = protected.snapshot();
    for event in [Event::AccessAccepted, Event::AccessUnavailable] {
        assert_eq!(
            protected.reduce(RevisionedEvent {
                revision: 2,
                session_generation: 1,
                event,
            }),
            Err(SessionError::InvalidTransition)
        );
        assert_eq!(protected.snapshot(), required);
    }
    reduce(&mut protected, 2, 1, Event::AccessAuthenticationSent);
    reduce(&mut protected, 3, 1, Event::AccessAccepted);
    let accepted = protected.snapshot();
    assert_eq!(
        protected.reduce(RevisionedEvent {
            revision: 4,
            session_generation: 1,
            event: Event::AccessAccepted,
        }),
        Err(SessionError::InvalidTransition)
    );
    assert_eq!(protected.snapshot(), accepted);

    let mut session = Session::default();
    reduce(&mut session, 1, 1, Event::Connected);
    let before = session.snapshot();
    for event in [
        Event::AccessRequired,
        Event::AccessAuthenticationSent,
        Event::AccessAccepted,
        Event::AccessUnavailable,
    ] {
        assert_eq!(
            session.reduce(RevisionedEvent {
                revision: 2,
                session_generation: 1,
                event,
            }),
            Err(SessionError::InvalidTransition)
        );
        assert_eq!(session.snapshot(), before);
    }
}

#[test]
fn protected_admission_fences_all_authorized_state_atomically() {
    let mut session = Session::default();
    reduce(&mut session, 1, 1, Event::AccessRequired);

    let events = vec![
        Event::PlayerStats {
            health: 1,
            health_max: 1,
        },
        Event::MapReset { map_generation: 1 },
        Event::EntityUpsert(Entity {
            handle: ObjectHandle {
                session_generation: 1,
                map_generation: 0,
                object_id: 1,
                object_generation: 1,
            },
            x: 0,
            y: 0,
            name: "entity".into(),
        }),
        Event::EntityRemoved(ObjectHandle {
            session_generation: 1,
            map_generation: 0,
            object_id: 1,
            object_generation: 1,
        }),
        Event::InventoryReplay(vec![]),
        Event::DialogReplaced("dialog".into()),
        Event::QuestReplaced("quest".into()),
        Event::Message("message".into()),
        Event::ActionResolved { request_id: 1 },
    ];

    for phase_transition in [None, Some(Event::AccessAuthenticationSent)] {
        if let Some(event) = phase_transition {
            reduce(&mut session, 2, 1, event);
        }
        let before = session.snapshot();
        for event in events.clone() {
            assert_eq!(
                session.reduce(RevisionedEvent {
                    revision: before.revision + 1,
                    session_generation: 1,
                    event,
                }),
                Err(SessionError::InvalidTransition)
            );
            assert_eq!(session.snapshot(), before);
        }
    }
}
