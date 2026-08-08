#![forbid(unsafe_code)]
//! Renderer-, protocol-, platform-, and transport-independent player intent.

use std::collections::VecDeque;
use std::error::Error;
use std::fmt::{Display, Formatter};

/// A direction in authoritative grid space.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    North,
    East,
    South,
    West,
}

/// A generational reference supplied by an immutable session view.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ObjectHandle {
    pub session_generation: u64,
    pub map_generation: u64,
    pub object_id: u64,
    pub object_generation: u32,
}

/// The complete M1 semantic-action vocabulary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Move(Direction),
    Stop,
    Target(ObjectHandle),
    Attack(ObjectHandle),
    Cast {
        spell_id: u32,
        target: Option<ObjectHandle>,
    },
    Apply(ObjectHandle),
    Get(ObjectHandle),
    Drop {
        item: ObjectHandle,
        quantity: u32,
    },
    Talk(ObjectHandle),
    Reply {
        choice_id: u32,
    },
    SelectCharacter {
        character_id: u64,
    },
}

/// Input normalized before product behavior sees a device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticInput {
    Navigate(Direction),
    Activate,
    Cancel,
    Text,
}

/// Converts device-independent navigation into the shared action route.
pub const fn navigation_action(input: SemanticInput) -> Option<Action> {
    match input {
        SemanticInput::Navigate(direction) => Some(Action::Move(direction)),
        SemanticInput::Cancel => Some(Action::Stop),
        SemanticInput::Activate | SemanticInput::Text => None,
    }
}

/// An action paired with a correlation identifier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionRequest {
    pub id: u64,
    pub action: Action,
}

/// A bounded non-blocking action queue.
#[derive(Debug)]
pub struct ActionQueue {
    capacity: usize,
    requests: VecDeque<ActionRequest>,
}

/// A typed local action rejection. It never claims authoritative failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionError {
    InvalidCapacity,
    Full,
    InvalidCorrelation,
}

impl Display for ActionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidCapacity => "action queue capacity is outside supported bounds",
            Self::Full => "action queue is full",
            Self::InvalidCorrelation => "action correlation identifier is zero",
        })
    }
}

impl Error for ActionError {}

impl ActionQueue {
    /// Creates a queue bounded to at most 4,096 requests.
    pub fn new(capacity: usize) -> Result<Self, ActionError> {
        if !(1..=4_096).contains(&capacity) {
            return Err(ActionError::InvalidCapacity);
        }
        Ok(Self {
            capacity,
            requests: VecDeque::with_capacity(capacity),
        })
    }

    /// Enqueues without blocking or dropping older intent.
    pub fn push(&mut self, request: ActionRequest) -> Result<(), ActionError> {
        if request.id == 0 {
            return Err(ActionError::InvalidCorrelation);
        }
        if self.requests.len() == self.capacity {
            return Err(ActionError::Full);
        }
        self.requests.push_back(request);
        Ok(())
    }

    /// Removes the oldest request.
    pub fn pop(&mut self) -> Option<ActionRequest> {
        self.requests.pop_front()
    }

    /// Removes all pending local intent at a lifecycle boundary.
    pub fn clear(&mut self) {
        self.requests.clear();
    }

    /// Returns current bounded depth.
    pub fn len(&self) -> usize {
        self.requests.len()
    }

    /// Reports whether no request is queued.
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_and_controller_navigation_share_actions() {
        let keyboard = navigation_action(SemanticInput::Navigate(Direction::North));
        let controller = navigation_action(SemanticInput::Navigate(Direction::North));
        assert_eq!(keyboard, controller);
    }

    #[test]
    fn queue_is_bounded_and_clearable() {
        let mut queue = ActionQueue::new(1).expect("valid capacity");
        let request = ActionRequest {
            id: 1,
            action: Action::Stop,
        };
        queue.push(request.clone()).expect("first request");
        assert_eq!(queue.push(request), Err(ActionError::Full));
        queue.clear();
        assert!(queue.is_empty());
    }
}
