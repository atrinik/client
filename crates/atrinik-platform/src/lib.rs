#![forbid(unsafe_code)]
//! SDL3 ownership boundary and deterministic headless substitutes.

use atrinik_actions::{Direction, SemanticInput};
use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformPathError {
    MissingEnvironment,
    RelativeEnvironment,
}

impl Display for PlatformPathError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::MissingEnvironment => "platform cache directory is unavailable",
            Self::RelativeEnvironment => "platform cache directory is not absolute",
        })
    }
}

impl Error for PlatformPathError {}

pub fn default_client_cache_root() -> Result<PathBuf, PlatformPathError> {
    #[cfg(target_os = "windows")]
    let root = cache_root_for(
        CachePlatform::Windows,
        std::env::var_os("LOCALAPPDATA").as_deref(),
        None,
        None,
    )?;

    #[cfg(target_os = "macos")]
    let root = cache_root_for(
        CachePlatform::MacOs,
        None,
        None,
        std::env::var_os("HOME").as_deref(),
    )?;

    #[cfg(all(unix, not(target_os = "macos")))]
    let root = cache_root_for(
        CachePlatform::Unix,
        None,
        std::env::var_os("XDG_CACHE_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )?;

    Ok(root)
}

#[derive(Clone, Copy)]
#[allow(dead_code)] // The shared pure path policy is tested on every host target.
enum CachePlatform {
    Windows,
    MacOs,
    Unix,
}

fn cache_root_for(
    platform: CachePlatform,
    local_app_data: Option<&std::ffi::OsStr>,
    xdg_cache_home: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, PlatformPathError> {
    let root = match platform {
        CachePlatform::Windows => {
            PathBuf::from(local_app_data.ok_or(PlatformPathError::MissingEnvironment)?)
                .join("Atrinik")
                .join("cache")
        }
        CachePlatform::MacOs => PathBuf::from(home.ok_or(PlatformPathError::MissingEnvironment)?)
            .join("Library")
            .join("Caches")
            .join("Atrinik"),
        CachePlatform::Unix => match xdg_cache_home {
            Some(value) => PathBuf::from(value).join("atrinik"),
            None => PathBuf::from(home.ok_or(PlatformPathError::MissingEnvironment)?)
                .join(".cache")
                .join("atrinik"),
        },
    };
    if !root.is_absolute() {
        return Err(PlatformPathError::RelativeEnvironment);
    }
    Ok(root)
}

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
    InvalidCapacity,
    EventQueueFull,
    Native(String),
}
impl Display for PlatformError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDimensions => f.write_str("logical window dimensions are invalid"),
            Self::InvalidScale => f.write_str("display scale is invalid"),
            Self::InvalidTransition => f.write_str("platform lifecycle transition is invalid"),
            Self::InvalidCapacity => {
                f.write_str("platform event capacity is outside supported bounds")
            }
            Self::EventQueueFull => f.write_str("platform event queue is full"),
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

#[derive(Clone, Copy, Debug, Default)]
pub struct HeadlessClock {
    elapsed: Duration,
}
impl HeadlessClock {
    pub fn advance(&mut self, delta: Duration) -> Result<(), PlatformError> {
        if delta > Duration::from_hours(24) {
            return Err(PlatformError::InvalidTransition);
        }
        self.elapsed = self
            .elapsed
            .checked_add(delta)
            .ok_or(PlatformError::InvalidTransition)?;
        Ok(())
    }
}
impl MonotonicClock for HeadlessClock {
    fn elapsed(&self) -> Duration {
        self.elapsed
    }
}

#[derive(Clone, Debug, Default)]
pub struct HeadlessClipboard {
    text: String,
}
impl Clipboard for HeadlessClipboard {
    fn read_text(&self) -> Result<String, PlatformError> {
        Ok(self.text.clone())
    }
    fn write_text(&mut self, value: &str) -> Result<(), PlatformError> {
        if value.len() > 1_048_576 || value.contains('\0') {
            return Err(PlatformError::InvalidTransition);
        }
        self.text.clear();
        self.text.push_str(value);
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct HeadlessAudio {
    connected: bool,
    volumes: BTreeMap<AudioCategory, u16>,
}
impl Default for HeadlessAudio {
    fn default() -> Self {
        Self {
            connected: true,
            volumes: BTreeMap::new(),
        }
    }
}
impl HeadlessAudio {
    pub fn lose(&mut self) {
        self.connected = false;
    }
    pub fn restore(&mut self) {
        self.connected = true;
    }
    pub fn volume(&self, category: AudioCategory) -> u16 {
        self.volumes.get(&category).copied().unwrap_or(1_000)
    }
}
impl AudioDevice for HeadlessAudio {
    fn set_category_volume(
        &mut self,
        category: AudioCategory,
        volume_milli: u16,
    ) -> Result<(), PlatformError> {
        if !self.connected || volume_milli > 1_000 {
            return Err(PlatformError::InvalidTransition);
        }
        self.volumes.insert(category, volume_milli);
        Ok(())
    }
    fn connected(&self) -> bool {
        self.connected
    }
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

pub struct HeadlessPlatform {
    state: PlatformState,
    events: VecDeque<PlatformTransition>,
    capacity: usize,
}

impl Default for HeadlessPlatform {
    fn default() -> Self {
        Self {
            state: PlatformState::default(),
            events: VecDeque::with_capacity(256),
            capacity: 256,
        }
    }
}
impl HeadlessPlatform {
    pub fn new(capacity: usize) -> Result<Self, PlatformError> {
        if !(1..=4_096).contains(&capacity) {
            return Err(PlatformError::InvalidCapacity);
        }
        Ok(Self {
            state: PlatformState::default(),
            events: VecDeque::with_capacity(capacity),
            capacity,
        })
    }
    pub fn apply(&mut self, transition: PlatformTransition) -> Result<(), PlatformError> {
        if self.events.len() == self.capacity {
            return Err(PlatformError::EventQueueFull);
        }
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
    use std::ffi::OsStr;
    #[test]
    fn devices_have_identical_navigation_semantics() {
        assert_eq!(
            semantic_input(RawInput::KeyLeft),
            semantic_input(RawInput::ControllerLeft)
        );
    }
    #[test]
    fn platform_cache_paths_are_absolute_distinct_and_fail_closed() {
        #[cfg(windows)]
        {
            assert_eq!(
                cache_root_for(
                    CachePlatform::Windows,
                    Some(OsStr::new("C:\\Users\\player\\AppData\\Local")),
                    None,
                    None,
                ),
                Ok(PathBuf::from(
                    "C:\\Users\\player\\AppData\\Local\\Atrinik\\cache",
                ))
            );
            assert_eq!(
                cache_root_for(CachePlatform::Windows, None, None, None),
                Err(PlatformPathError::MissingEnvironment)
            );
            assert_eq!(
                cache_root_for(
                    CachePlatform::Windows,
                    Some(OsStr::new("relative")),
                    None,
                    None,
                ),
                Err(PlatformPathError::RelativeEnvironment)
            );
        }
        #[cfg(not(windows))]
        {
            assert_eq!(
                cache_root_for(
                    CachePlatform::MacOs,
                    None,
                    None,
                    Some(OsStr::new("/Users/player")),
                ),
                Ok(PathBuf::from("/Users/player/Library/Caches/Atrinik"))
            );
            assert_eq!(
                cache_root_for(
                    CachePlatform::Unix,
                    None,
                    Some(OsStr::new("/cache/player")),
                    Some(OsStr::new("/home/player")),
                ),
                Ok(PathBuf::from("/cache/player/atrinik"))
            );
            assert_eq!(
                cache_root_for(
                    CachePlatform::Unix,
                    None,
                    None,
                    Some(OsStr::new("/home/player")),
                ),
                Ok(PathBuf::from("/home/player/.cache/atrinik"))
            );
            assert_eq!(
                cache_root_for(CachePlatform::Unix, None, None, None),
                Err(PlatformPathError::MissingEnvironment)
            );
            assert_eq!(
                cache_root_for(
                    CachePlatform::Unix,
                    None,
                    Some(OsStr::new("relative")),
                    None,
                ),
                Err(PlatformPathError::RelativeEnvironment)
            );
        }
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
    #[test]
    fn event_queue_fails_before_state_mutation() {
        let mut platform = HeadlessPlatform::new(1).expect("capacity");
        platform
            .apply(PlatformTransition::WindowCreated)
            .expect("first event");
        let before = platform.state();
        assert_eq!(
            platform.apply(PlatformTransition::Fullscreen(true)),
            Err(PlatformError::EventQueueFull)
        );
        assert_eq!(platform.state(), before);
    }
    #[test]
    fn headless_services_enforce_time_text_audio_and_device_loss_bounds() {
        let mut clock = HeadlessClock::default();
        clock.advance(Duration::from_millis(5)).expect("time");
        assert_eq!(clock.elapsed(), Duration::from_millis(5));
        assert_eq!(
            clock.advance(Duration::from_secs(86_401)),
            Err(PlatformError::InvalidTransition)
        );
        let mut clipboard = HeadlessClipboard::default();
        clipboard.write_text("text").expect("clipboard");
        assert_eq!(clipboard.read_text().expect("read"), "text");
        assert_eq!(
            clipboard.write_text("bad\0text"),
            Err(PlatformError::InvalidTransition)
        );
        let mut audio = HeadlessAudio::default();
        audio
            .set_category_volume(AudioCategory::Music, 500)
            .expect("volume");
        assert_eq!(audio.volume(AudioCategory::Music), 500);
        audio.lose();
        assert_eq!(
            audio.set_category_volume(AudioCategory::Music, 400),
            Err(PlatformError::InvalidTransition)
        );
        audio.restore();
        assert!(audio.connected());
    }
}
