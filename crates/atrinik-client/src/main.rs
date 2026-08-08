#![forbid(unsafe_code)]

use atrinik_config_cache::StorageClass;
use atrinik_protocol_adapter::{Envelope, PROTOCOL_CONTRACT, ValidatedMessage, into_domain};
use atrinik_scene_adapter::RENDERER_CONTRACT;
use atrinik_session::Session;
use atrinik_ui_model::model;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    match std::env::args().nth(1).as_deref() {
        Some("version") => {
            println!(
                "atrinik-client {} rust={} target={} protocol={} renderer={}",
                option_env!("ATRINIK_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")),
                option_env!("ATRINIK_RUST_VERSION").unwrap_or("rust-1.97.1"),
                std::env::consts::OS,
                PROTOCOL_CONTRACT,
                RENDERER_CONTRACT
            );
            Ok(())
        }
        Some("headless") => headless(),
        Some("window") => {
            atrinik_platform::sdl::window_harness(1)?;
            Ok(())
        }
        _ => Err("usage: atrinik-client version|headless|window".into()),
    }
}

fn headless() -> Result<(), Box<dyn Error>> {
    let resource_boundary = StorageClass::ResourceCache.directory();
    let mut session = Session::default();
    for envelope in [
        Envelope {
            revision: 1,
            session_generation: 1,
            payload_bytes: 1,
            message: ValidatedMessage::Connected,
        },
        Envelope {
            revision: 2,
            session_generation: 1,
            payload_bytes: 1,
            message: ValidatedMessage::EnteredWorld,
        },
    ] {
        session.reduce(into_domain(envelope)?)?;
    }
    let snapshot = session.snapshot();
    let ui = model(&snapshot);
    println!(
        "headless revision={} generation={} playing={} resource_root={}",
        snapshot.revision, snapshot.session_generation, ui.playing, resource_boundary
    );
    Ok(())
}
