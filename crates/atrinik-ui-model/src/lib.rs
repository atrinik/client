#![forbid(unsafe_code)]
//! Immutable presentation models derived from session snapshots.

use atrinik_actions::{Action, Direction};
use atrinik_session::{Phase, Snapshot};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiModel {
    pub connected: bool,
    pub playing: bool,
    pub health_label: String,
    pub message_count: usize,
    pub can_act: bool,
}

pub fn model(snapshot: &Snapshot) -> UiModel {
    UiModel {
        connected: snapshot.phase != Phase::Disconnected,
        playing: snapshot.phase == Phase::Playing,
        health_label: format!(
            "{} / {}",
            snapshot.player.health, snapshot.player.health_max
        ),
        message_count: snapshot.messages.len(),
        can_act: snapshot.phase == Phase::Playing,
    }
}

pub const fn navigation(direction: Direction) -> Action {
    Action::Move(direction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atrinik_session::{Player, Snapshot};
    #[test]
    fn ui_reads_a_copy_and_emits_semantic_actions() {
        let snapshot = Snapshot {
            revision: 1,
            session_generation: 1,
            map_generation: 1,
            phase: Phase::Playing,
            player: Player {
                health: 4,
                health_max: 5,
                movement: None,
                target: None,
            },
            entities: vec![],
            inventory: vec![],
            dialog: String::new(),
            quest: String::new(),
            messages: vec!["safe".into()],
            pending_action_ids: vec![],
        };
        assert_eq!(model(&snapshot).health_label, "4 / 5");
        assert_eq!(navigation(Direction::East), Action::Move(Direction::East));
    }
}
