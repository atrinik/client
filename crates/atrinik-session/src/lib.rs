#![forbid(unsafe_code)]
//! Pure, single-owner, revisioned client session state.

use atrinik_actions::{Action, ActionRequest, Direction, ObjectHandle};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error;
use std::fmt::{Display, Formatter};

const MAX_TEXT_BYTES: usize = 512;
const MAX_ENTITIES: usize = 65_536;
const MAX_INVENTORY: usize = 4_096;
const MAX_MESSAGES: usize = 1_024;
const MAX_PENDING_ACTIONS: usize = 256;
const MAX_COORDINATE: i32 = 1_000_000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Phase {
    #[default]
    Disconnected,
    Connected,
    Playing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entity {
    pub handle: ObjectHandle,
    pub x: i32,
    pub y: i32,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    pub handle: ObjectHandle,
    pub quantity: u32,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Player {
    pub health: u32,
    pub health_max: u32,
    pub movement: Option<Direction>,
    pub target: Option<ObjectHandle>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    Connected,
    EnteredWorld,
    Disconnected,
    PlayerStats { health: u32, health_max: u32 },
    MapReset { map_generation: u64 },
    EntityUpsert(Entity),
    EntityRemoved(ObjectHandle),
    InventoryReplay(Vec<Item>),
    DialogReplaced(String),
    QuestReplaced(String),
    Message(String),
    ActionResolved { request_id: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionedEvent {
    pub revision: u64,
    pub session_generation: u64,
    pub event: Event,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub revision: u64,
    pub session_generation: u64,
    pub map_generation: u64,
    pub phase: Phase,
    pub player: Player,
    pub entities: Vec<Entity>,
    pub inventory: Vec<Item>,
    pub dialog: String,
    pub quest: String,
    pub messages: Vec<String>,
    pub pending_action_ids: Vec<u64>,
}

pub trait ActionSink {
    fn send(&mut self, request: &ActionRequest) -> Result<(), SessionError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionError {
    RevisionGap,
    StaleSession,
    StaleHandle,
    InvalidTransition,
    InvalidValue,
    CollectionLimit,
    TextLimit,
    PendingLimit,
    DuplicateRequest,
    TransportRejected,
}

impl Display for SessionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::RevisionGap => "session revision is stale, duplicate, or has a gap",
            Self::StaleSession => "event belongs to a stale session generation",
            Self::StaleHandle => "object handle is stale",
            Self::InvalidTransition => "session lifecycle transition is invalid",
            Self::InvalidValue => "session value is invalid",
            Self::CollectionLimit => "session collection exceeds its bound",
            Self::TextLimit => "server-controlled text exceeds its bound",
            Self::PendingLimit => "pending action limit reached",
            Self::DuplicateRequest => "action correlation identifier is duplicated",
            Self::TransportRejected => "transport rejected the local action request",
        })
    }
}
impl Error for SessionError {}

#[derive(Clone, Debug)]
pub struct Session {
    revision: u64,
    generation: u64,
    map_generation: u64,
    phase: Phase,
    player: Player,
    entities: BTreeMap<u64, Entity>,
    entity_generations: BTreeMap<u64, u32>,
    inventory: BTreeMap<u64, Item>,
    inventory_generations: BTreeMap<u64, u32>,
    dialog: String,
    quest: String,
    messages: VecDeque<String>,
    pending: BTreeMap<u64, Action>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            revision: 0,
            generation: 0,
            map_generation: 0,
            phase: Phase::Disconnected,
            player: Player {
                health: 0,
                health_max: 0,
                movement: None,
                target: None,
            },
            entities: BTreeMap::new(),
            entity_generations: BTreeMap::new(),
            inventory: BTreeMap::new(),
            inventory_generations: BTreeMap::new(),
            dialog: String::new(),
            quest: String::new(),
            messages: VecDeque::new(),
            pending: BTreeMap::new(),
        }
    }
}

impl Session {
    pub fn reduce(&mut self, incoming: RevisionedEvent) -> Result<(), SessionError> {
        if incoming.revision
            != self
                .revision
                .checked_add(1)
                .ok_or(SessionError::RevisionGap)?
        {
            return Err(SessionError::RevisionGap);
        }
        let connecting = matches!(incoming.event, Event::Connected);
        if connecting {
            if self.phase != Phase::Disconnected || incoming.session_generation <= self.generation {
                return Err(SessionError::InvalidTransition);
            }
        } else if incoming.session_generation != self.generation {
            return Err(SessionError::StaleSession);
        }
        self.preflight(&incoming.event)?;
        self.commit(incoming.event);
        self.revision = incoming.revision;
        if connecting {
            self.generation = incoming.session_generation;
        }
        Ok(())
    }

    fn preflight(&self, event: &Event) -> Result<(), SessionError> {
        match event {
            Event::Connected if self.phase != Phase::Disconnected => {
                Err(SessionError::InvalidTransition)
            }
            Event::EnteredWorld if self.phase != Phase::Connected => {
                Err(SessionError::InvalidTransition)
            }
            Event::Disconnected if self.phase == Phase::Disconnected => {
                Err(SessionError::InvalidTransition)
            }
            Event::PlayerStats { health, health_max }
                if *health_max == 0 || health > health_max =>
            {
                Err(SessionError::InvalidValue)
            }
            Event::MapReset { map_generation } if *map_generation <= self.map_generation => {
                Err(SessionError::InvalidValue)
            }
            Event::EntityUpsert(entity) => {
                validate_text(&entity.name)?;
                if !self.handle_generation_valid(entity.handle) {
                    return Err(SessionError::StaleHandle);
                }
                if entity.x.unsigned_abs() > MAX_COORDINATE.unsigned_abs()
                    || entity.y.unsigned_abs() > MAX_COORDINATE.unsigned_abs()
                {
                    return Err(SessionError::InvalidValue);
                }
                if let Some(current) = self.entities.get(&entity.handle.object_id) {
                    if current.handle.object_generation != entity.handle.object_generation {
                        return Err(SessionError::StaleHandle);
                    }
                } else {
                    if self.entities.len() == MAX_ENTITIES {
                        return Err(SessionError::CollectionLimit);
                    }
                    if self
                        .entity_generations
                        .get(&entity.handle.object_id)
                        .is_some_and(|generation| entity.handle.object_generation <= *generation)
                    {
                        return Err(SessionError::StaleHandle);
                    }
                }
                Ok(())
            }
            Event::EntityRemoved(handle) if !self.handle_is_current(*handle) => {
                Err(SessionError::StaleHandle)
            }
            Event::InventoryReplay(items) => {
                if items.len() > MAX_INVENTORY {
                    return Err(SessionError::CollectionLimit);
                }
                let mut ids = BTreeSet::new();
                for item in items {
                    validate_text(&item.name)?;
                    if item.quantity == 0
                        || !self.inventory_generation_valid(item.handle)
                        || !ids.insert(item.handle.object_id)
                    {
                        return Err(SessionError::InvalidValue);
                    }
                    if let Some(current) = self.inventory.get(&item.handle.object_id) {
                        if current.handle.object_generation != item.handle.object_generation {
                            return Err(SessionError::StaleHandle);
                        }
                    } else if self
                        .inventory_generations
                        .get(&item.handle.object_id)
                        .is_some_and(|generation| item.handle.object_generation <= *generation)
                    {
                        return Err(SessionError::StaleHandle);
                    }
                }
                Ok(())
            }
            Event::DialogReplaced(text) | Event::QuestReplaced(text) | Event::Message(text) => {
                validate_text(text)
            }
            Event::ActionResolved { request_id } if !self.pending.contains_key(request_id) => {
                Err(SessionError::InvalidValue)
            }
            _ => Ok(()),
        }
    }

    fn commit(&mut self, event: Event) {
        match event {
            Event::Connected => {
                self.reset_transient();
                self.phase = Phase::Connected;
            }
            Event::EnteredWorld => self.phase = Phase::Playing,
            Event::Disconnected => {
                self.reset_transient();
                self.phase = Phase::Disconnected;
            }
            Event::PlayerStats { health, health_max } => {
                self.player.health = health;
                self.player.health_max = health_max;
            }
            Event::MapReset { map_generation } => {
                self.map_generation = map_generation;
                self.entities.clear();
                self.entity_generations.clear();
                self.player.target = None;
            }
            Event::EntityUpsert(entity) => {
                self.entity_generations
                    .insert(entity.handle.object_id, entity.handle.object_generation);
                self.entities.insert(entity.handle.object_id, entity);
            }
            Event::EntityRemoved(handle) => {
                self.entities.remove(&handle.object_id);
                if self.player.target == Some(handle) {
                    self.player.target = None;
                }
            }
            Event::InventoryReplay(items) => {
                for item in &items {
                    self.inventory_generations
                        .insert(item.handle.object_id, item.handle.object_generation);
                }
                self.inventory = items
                    .into_iter()
                    .map(|item| (item.handle.object_id, item))
                    .collect();
            }
            Event::DialogReplaced(text) => self.dialog = text,
            Event::QuestReplaced(text) => self.quest = text,
            Event::Message(text) => {
                if self.messages.len() == MAX_MESSAGES {
                    self.messages.pop_front();
                }
                self.messages.push_back(text);
            }
            Event::ActionResolved { request_id } => {
                self.pending.remove(&request_id);
            }
        }
    }

    pub fn dispatch(
        &mut self,
        request: ActionRequest,
        sink: &mut dyn ActionSink,
    ) -> Result<(), SessionError> {
        let phase_is_valid = (matches!(request.action, Action::SelectCharacter { .. })
            && self.phase == Phase::Connected)
            || (!matches!(request.action, Action::SelectCharacter { .. })
                && self.phase == Phase::Playing);
        if !phase_is_valid {
            return Err(SessionError::InvalidTransition);
        }
        if request.id == 0 || self.pending.contains_key(&request.id) {
            return Err(SessionError::DuplicateRequest);
        }
        if self.pending.len() == MAX_PENDING_ACTIONS {
            return Err(SessionError::PendingLimit);
        }
        self.validate_action(&request.action)?;
        sink.send(&request)
            .map_err(|_| SessionError::TransportRejected)?;
        if let Action::Move(direction) = request.action {
            self.player.movement = Some(direction);
        }
        if matches!(request.action, Action::Stop) {
            self.player.movement = None;
        }
        if let Action::Target(handle) = request.action {
            self.player.target = Some(handle);
        }
        self.pending.insert(request.id, request.action);
        Ok(())
    }

    fn validate_action(&self, action: &Action) -> Result<(), SessionError> {
        match action {
            Action::Cast { spell_id: 0, .. }
            | Action::Reply { choice_id: 0 }
            | Action::SelectCharacter { character_id: 0 } => {
                return Err(SessionError::InvalidValue);
            }
            _ => {}
        }
        let handle = match action {
            Action::Target(value)
            | Action::Attack(value)
            | Action::Apply(value)
            | Action::Get(value)
            | Action::Talk(value) => Some(*value),
            Action::Cast { target, .. } => *target,
            Action::Drop { item, quantity } => {
                if *quantity == 0 {
                    return Err(SessionError::InvalidValue);
                }
                Some(*item)
            }
            Action::Move(_)
            | Action::Stop
            | Action::Reply { .. }
            | Action::SelectCharacter { .. } => None,
        };
        if handle.is_some_and(|value| {
            !self.handle_is_current(value) && !self.inventory_handle_is_current(value)
        }) {
            return Err(SessionError::StaleHandle);
        }
        Ok(())
    }

    fn handle_generation_valid(&self, handle: ObjectHandle) -> bool {
        handle.object_id != 0
            && handle.object_generation != 0
            && handle.session_generation == self.generation
            && handle.map_generation == self.map_generation
    }
    fn inventory_generation_valid(&self, handle: ObjectHandle) -> bool {
        handle.object_id != 0
            && handle.object_generation != 0
            && handle.session_generation == self.generation
            && handle.map_generation == 0
    }
    fn handle_is_current(&self, handle: ObjectHandle) -> bool {
        self.entities
            .get(&handle.object_id)
            .is_some_and(|entity| entity.handle == handle)
    }
    fn inventory_handle_is_current(&self, handle: ObjectHandle) -> bool {
        self.inventory
            .get(&handle.object_id)
            .is_some_and(|item| item.handle == handle)
    }
    fn reset_transient(&mut self) {
        self.map_generation = 0;
        self.entities.clear();
        self.entity_generations.clear();
        self.inventory.clear();
        self.inventory_generations.clear();
        self.dialog.clear();
        self.quest.clear();
        self.messages.clear();
        self.pending.clear();
        self.player = Player {
            health: 0,
            health_max: 0,
            movement: None,
            target: None,
        };
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            revision: self.revision,
            session_generation: self.generation,
            map_generation: self.map_generation,
            phase: self.phase,
            player: self.player.clone(),
            entities: self.entities.values().cloned().collect(),
            inventory: self.inventory.values().cloned().collect(),
            dialog: self.dialog.clone(),
            quest: self.quest.clone(),
            messages: self.messages.iter().cloned().collect(),
            pending_action_ids: self.pending.keys().copied().collect(),
        }
    }
}

fn validate_text(text: &str) -> Result<(), SessionError> {
    if text.len() > MAX_TEXT_BYTES || text.contains('\0') {
        Err(SessionError::TextLimit)
    } else {
        Ok(())
    }
}
