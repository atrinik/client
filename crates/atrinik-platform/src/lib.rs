#![forbid(unsafe_code)]
//! SDL3 ownership boundary and deterministic headless substitutes.

use atrinik_actions::{Direction, SemanticInput};
use std::collections::VecDeque;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RawInput {
    KeyUp,
    KeyDown,
    KeyLeft,
    KeyRight,
    ControllerUp,
    ControllerDown,
    ControllerLeft,
    ControllerRight,
    Activate,
    Cancel,
    Text,
}

pub const fn semantic_input(input: RawInput) -> SemanticInput {
    match input {
        RawInput::KeyUp | RawInput::ControllerUp => SemanticInput::Navigate(Direction::North),
        RawInput::KeyRight | RawInput::ControllerRight => SemanticInput::Navigate(Direction::East),
        RawInput::KeyDown | RawInput::ControllerDown => SemanticInput::Navigate(Direction::South),
        RawInput::KeyLeft | RawInput::ControllerLeft => SemanticInput::Navigate(Direction::West),
        RawInput::Activate => SemanticInput::Activate,
        RawInput::Cancel => SemanticInput::Cancel,
        RawInput::Text => SemanticInput::Text,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformTransition {
    WindowCreated,
    Resized {
        logical_width: u32,
        logical_height: u32,
        scale_milli: u32,
    },
    Fullscreen(bool),
    Focused(bool),
    Suspended(bool),
    InputConnected,
    InputLost,
    AudioConnected,
    AudioLost,
    Shutdown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlatformError {
    InvalidDimensions,
    InvalidScale,
    InvalidTransition,
    Native(String),
}
impl Display for PlatformError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDimensions => f.write_str("logical window dimensions are invalid"),
            Self::InvalidScale => f.write_str("display scale is invalid"),
            Self::InvalidTransition => f.write_str("platform lifecycle transition is invalid"),
            Self::Native(message) => write!(f, "SDL3 platform error: {message}"),
        }
    }
}
impl Error for PlatformError {}

pub trait MonotonicClock {
    fn elapsed(&self) -> Duration;
}
pub trait Clipboard {
    fn read_text(&self) -> Result<String, PlatformError>;
    fn write_text(&mut self, value: &str) -> Result<(), PlatformError>;
}
pub trait AudioDevice {
    fn set_category_volume(
        &mut self,
        category: AudioCategory,
        volume_milli: u16,
    ) -> Result<(), PlatformError>;
    fn connected(&self) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AudioCategory {
    Master,
    Music,
    Effects,
    Interface,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct PlatformState {
    pub window: bool,
    pub width: u32,
    pub height: u32,
    pub scale_milli: u32,
    pub fullscreen: bool,
    pub focused: bool,
    pub suspended: bool,
    pub input_connected: bool,
    pub audio_connected: bool,
    pub shutdown: bool,
}

#[derive(Default)]
pub struct HeadlessPlatform {
    state: PlatformState,
    events: VecDeque<PlatformTransition>,
}
impl HeadlessPlatform {
    pub fn apply(&mut self, transition: PlatformTransition) -> Result<(), PlatformError> {
        match transition {
            PlatformTransition::WindowCreated if !self.state.window && !self.state.shutdown => {
                self.state.window = true;
                self.state.width = 800;
                self.state.height = 600;
                self.state.scale_milli = 1_000;
            }
            PlatformTransition::Resized {
                logical_width,
                logical_height,
                scale_milli,
            } if self.state.window
                && logical_width > 0
                && logical_height > 0
                && logical_width <= 16_384
                && logical_height <= 16_384
                && (250..=8_000).contains(&scale_milli) =>
            {
                self.state.width = logical_width;
                self.state.height = logical_height;
                self.state.scale_milli = scale_milli;
            }
            PlatformTransition::Fullscreen(value) if self.state.window => {
                self.state.fullscreen = value;
            }
            PlatformTransition::Focused(value) if self.state.window => self.state.focused = value,
            PlatformTransition::Suspended(value) => self.state.suspended = value,
            PlatformTransition::InputConnected => self.state.input_connected = true,
            PlatformTransition::InputLost => self.state.input_connected = false,
            PlatformTransition::AudioConnected => self.state.audio_connected = true,
            PlatformTransition::AudioLost => self.state.audio_connected = false,
            PlatformTransition::Shutdown if !self.state.shutdown => {
                self.state = PlatformState {
                    shutdown: true,
                    ..PlatformState::default()
                };
            }
            PlatformTransition::Resized {
                logical_width: 0, ..
            }
            | PlatformTransition::Resized {
                logical_height: 0, ..
            } => return Err(PlatformError::InvalidDimensions),
            PlatformTransition::Resized { scale_milli, .. }
                if !(250..=8_000).contains(&scale_milli) =>
            {
                return Err(PlatformError::InvalidScale);
            }
            _ => return Err(PlatformError::InvalidTransition),
        }
        self.events.push_back(transition);
        Ok(())
    }
    pub fn state(&self) -> PlatformState {
        self.state.clone()
    }
    pub fn pop_event(&mut self) -> Option<PlatformTransition> {
        self.events.pop_front()
    }
}

#[cfg(feature = "sdl-runtime")]
pub mod sdl {
    use super::PlatformError;
    pub fn window_harness(iterations: usize) -> Result<(), PlatformError> {
        if !(1..=64).contains(&iterations) {
            return Err(PlatformError::InvalidTransition);
        }
        for _ in 0..iterations {
            let context = sdl3::init().map_err(|error| PlatformError::Native(error.to_string()))?;
            let video = context
                .video()
                .map_err(|error| PlatformError::Native(error.to_string()))?;
            let mut window = video
                .window("Atrinik", 800, 600)
                .position_centered()
                .resizable()
                .build()
                .map_err(|error| PlatformError::Native(error.to_string()))?;
            window
                .set_size(1_024, 768)
                .map_err(|error| PlatformError::Native(error.to_string()))?;
            drop(window);
            drop(video);
            drop(context);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn devices_have_identical_navigation_semantics() {
        assert_eq!(
            semantic_input(RawInput::KeyLeft),
            semantic_input(RawInput::ControllerLeft)
        );
    }
    #[test]
    fn lifecycle_handles_resize_hotplug_loss_and_shutdown() {
        let mut platform = HeadlessPlatform::default();
        for event in [
            PlatformTransition::WindowCreated,
            PlatformTransition::Resized {
                logical_width: 1_200,
                logical_height: 800,
                scale_milli: 1_500,
            },
            PlatformTransition::Fullscreen(true),
            PlatformTransition::Focused(true),
            PlatformTransition::InputConnected,
            PlatformTransition::InputLost,
            PlatformTransition::AudioConnected,
            PlatformTransition::AudioLost,
            PlatformTransition::Shutdown,
        ] {
            platform.apply(event).expect("transition");
        }
        assert!(platform.state().shutdown);
        assert_eq!(
            platform.apply(PlatformTransition::WindowCreated),
            Err(PlatformError::InvalidTransition)
        );
    }
}
