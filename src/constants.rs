//! Client version the bot presents to Growtopia.
//!
//! The game server rejects an outdated client with an `UPDATE REQUIRED` message,
//! so these move every few weeks. They are read once at start-up from
//! `data/version.json`, falling back to the values below, so a bump needs an edit
//! and a restart rather than a rebuild:
//!
//! ```json
//! { "game_version": "5.57", "protocol": 225, "fhash": -716928004 }
//! ```

use std::sync::OnceLock;

use serde::Deserialize;

const DEFAULT_GAME_VERSION: &str = "5.57";
const DEFAULT_PROTOCOL: u32 = 225;
const DEFAULT_FHASH: i32 = -716928004;

#[derive(Deserialize)]
struct VersionFile {
    game_version: Option<String>,
    protocol: Option<u32>,
    fhash: Option<i32>,
}

struct Version {
    game_version: String,
    protocol: u32,
    fhash: i32,
}

fn version() -> &'static Version {
    static VERSION: OnceLock<Version> = OnceLock::new();
    VERSION.get_or_init(|| {
        let path = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("data")
            .join("version.json");

        let file: Option<VersionFile> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| match serde_json::from_str(&raw) {
                Ok(v) => Some(v),
                Err(e) => {
                    println!("[Version] {} is not valid JSON: {e}", path.display());
                    None
                }
            });

        let v = Version {
            game_version: file
                .as_ref()
                .and_then(|f| f.game_version.clone())
                .unwrap_or_else(|| DEFAULT_GAME_VERSION.to_string()),
            protocol: file
                .as_ref()
                .and_then(|f| f.protocol)
                .unwrap_or(DEFAULT_PROTOCOL),
            fhash: file.as_ref().and_then(|f| f.fhash).unwrap_or(DEFAULT_FHASH),
        };

        println!(
            "[Version] game {} protocol {} fhash {}{}",
            v.game_version,
            v.protocol,
            v.fhash,
            if file.is_some() { " (data/version.json)" } else { "" }
        );
        v
    })
}

pub fn game_version() -> &'static str {
    &version().game_version
}

pub fn protocol() -> u32 {
    version().protocol
}

pub fn fhash() -> i32 {
    version().fhash
}
