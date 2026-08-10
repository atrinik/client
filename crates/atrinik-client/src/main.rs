#![forbid(unsafe_code)]

use atrinik_config_cache::StorageClass;
use atrinik_directory::cache::FileDirectoryCache;
use atrinik_directory::transport::UreqDirectoryTransport;
use atrinik_directory::{DirectoryService, DirectoryView};
use atrinik_platform::default_client_cache_root;
use atrinik_protocol_adapter::directory::InstalledCompatibility;
use atrinik_protocol_adapter::{Envelope, PROTOCOL_CONTRACT, ValidatedMessage, into_domain};
use atrinik_scene_adapter::RENDERER_CONTRACT;
use atrinik_session::Session;
use atrinik_ui_model::model;
use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    match arguments.next().as_deref() {
        None | Some("directory") => {
            if arguments.next().is_some() {
                return Err("unexpected directory argument".into());
            }
            directory()
        }
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
            let iterations = arguments
                .next()
                .map_or(Ok(8), |value| value.parse::<usize>())?;
            if arguments.next().is_some() {
                return Err("unexpected window argument".into());
            }
            atrinik_platform::sdl::window_harness(iterations)?;
            Ok(())
        }
        _ => Err("usage: atrinik-client [directory]|version|headless|window".into()),
    }
}

fn directory() -> Result<(), Box<dyn Error>> {
    let cache_root = default_client_cache_root()?.join(StorageClass::DirectoryCache.directory());
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let mut service = DirectoryService::new(
        UreqDirectoryTransport::new(),
        FileDirectoryCache::new(cache_root),
        InstalledCompatibility::Unavailable,
    );
    print_directory_view(&service.refresh(now));
    Ok(())
}

fn print_directory_view(view: &DirectoryView) {
    let compatible = view
        .snapshot
        .as_ref()
        .map_or(0, |snapshot| snapshot.servers.len());
    let incompatible = view
        .snapshot
        .as_ref()
        .map_or(0, |snapshot| snapshot.incompatible_servers);
    let notice = view.notice.map_or("none", |value| value.code());
    println!(
        "{}; compatible={compatible}; hidden-incompatible={incompatible}; notice={notice}",
        view.message()
    );
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
