//! Persistent per-account device identity.
//!
//! The login payload carries `rid`, `mac` and `wk` — values a real client derives
//! from the machine it runs on, so they stay the same across logins. Generating
//! them per `Bot` instance makes every restart look like a brand new device for the
//! same account. Identities are stored in `data/devices.json` (next to `user.json`)
//! and reused from then on.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::protocol::crypto::random_hex;

fn store_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("data")
        .join("devices.json")
}

/// Vendor OUIs of real hardware. `random_mac()` used to emit a `02:` prefix, which
/// marks the address as locally administered — rare on an actual player's machine.
const OUIS: &[&str] = &[
    "A4:83:E7", // Apple
    "3C:97:0E", // Wistron
    "F0:25:B7", // Samsung
    "B0:83:FE", // Dell
    "1C:87:2C", // ASUSTek
    "8C:85:90", // Apple
    "D8:9E:F3", // Dell
];

/// Serializes read-modify-write cycles when several bots spawn at once.
static FILE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub rid: String,
    pub mac: String,
    pub wk: String,
    /// Hashed into the login packet's `hash2` field.
    pub hash2_seed: String,
    /// Fields below describe the account and the client build. Every payload of a
    /// session has to carry the same values: the dashboard request, the check-token
    /// call and the in-game redirect packet all used to hardcode different ones, so
    /// a single account claimed three countries and three ages within minutes.
    /// Edit `country` to match the exit IP this account logs in from.
    #[serde(default = "default_country")]
    pub country: String,
    #[serde(default = "default_player_age")]
    pub player_age: u8,
    #[serde(default = "default_gdpr")]
    pub gdpr: u8,
    #[serde(default = "default_cbits")]
    pub cbits: u32,
    #[serde(default = "default_fz")]
    pub fz: i64,
    #[serde(default = "default_zf")]
    pub zf: i64,
    /// Total playtime in seconds, as last reported by the server's Track packet.
    /// Login payloads used to send 0 here while the server knew the real figure.
    #[serde(default)]
    pub total_playtime: u64,
}

fn default_country() -> String {
    "us".to_string()
}

fn default_player_age() -> u8 {
    20
}

fn default_gdpr() -> u8 {
    1
}

fn default_cbits() -> u32 {
    1024
}

fn default_fz() -> i64 {
    22243512
}

fn default_zf() -> i64 {
    31631978
}

impl DeviceIdentity {
    /// A fresh identity. Callers normally go through [`load_or_create`]; this is
    /// public so other modules can build one in tests.
    pub fn generate() -> Self {
        let mut rng = rand::rng();
        let oui = OUIS[rng.random_range(0..OUIS.len())];
        Self {
            rid: random_hex(32),
            mac: format!(
                "{oui}:{:02X}:{:02X}:{:02X}",
                rng.random::<u8>(),
                rng.random::<u8>(),
                rng.random::<u8>(),
            ),
            wk: random_hex(32),
            hash2_seed: random_hex(16),
            country: default_country(),
            // A real account carries whatever age its owner entered, so vary it per
            // identity instead of every bot reporting the same number.
            player_age: rng.random_range(18..=35),
            gdpr: default_gdpr(),
            cbits: default_cbits(),
            fz: default_fz(),
            zf: default_zf(),
            total_playtime: 0,
        }
    }
}

/// Returns the identity stored under `key`, generating and saving one if absent.
/// `key` is the account username for legacy login, or the rid for ltoken login.
pub fn load_or_create(key: &str) -> DeviceIdentity {
    with_store(key, None)
}

/// Same as [`load_or_create`], but keeps `rid`/`mac`/`wk` supplied by the caller —
/// used by ltoken login, where those three come from the token string and only
/// `hash2_seed` needs to persist.
pub fn load_or_create_with(key: &str, rid: &str, mac: &str, wk: &str) -> DeviceIdentity {
    with_store(key, Some((rid.to_string(), mac.to_string(), wk.to_string())))
}

fn with_store(key: &str, given: Option<(String, String, String)>) -> DeviceIdentity {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut store = read_store();
    let (identity, dirty) = resolve(&mut store, key, given);
    if dirty {
        write_store(&store);
    }
    identity
}

/// Looks `key` up, filling in a generated identity or the caller's values as needed.
/// Returns the identity and whether `store` changed and must be written back.
fn resolve(
    store: &mut BTreeMap<String, DeviceIdentity>,
    key: &str,
    given: Option<(String, String, String)>,
) -> (DeviceIdentity, bool) {
    let mut identity = store.get(key).cloned().unwrap_or_else(DeviceIdentity::generate);
    let mut dirty = !store.contains_key(key);

    if let Some((rid, mac, wk)) = given {
        if identity.rid != rid || identity.mac != mac || identity.wk != wk {
            identity.rid = rid;
            identity.mac = mac;
            identity.wk = wk;
            dirty = true;
        }
    }

    if dirty {
        store.insert(key.to_string(), identity.clone());
    }
    (identity, dirty)
}

/// Records the playtime the server reported for `key`. No-op when it is not larger
/// than what is already stored, so a truncated or missing figure cannot walk it back.
pub fn store_playtime(key: &str, seconds: u64) {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut store = read_store();
    let Some(entry) = store.get_mut(key) else {
        return;
    };
    if entry.total_playtime >= seconds {
        return;
    }
    entry.total_playtime = seconds;
    write_store(&store);
}

fn read_store() -> BTreeMap<String, DeviceIdentity> {
    let path = store_path();
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return BTreeMap::new(),
        Err(e) => {
            println!("[Device] failed to read {}: {e} - using a fresh identity", path.display());
            return BTreeMap::new();
        }
    };

    serde_json::from_str(&data).unwrap_or_else(|e| {
        println!("[Device] failed to parse {}: {e} - using a fresh identity", path.display());
        BTreeMap::new()
    })
}

fn write_store(store: &BTreeMap<String, DeviceIdentity>) {
    let path = store_path();
    let json = match serde_json::to_string_pretty(store) {
        Ok(j) => j,
        Err(e) => {
            println!("[Device] failed to serialize device store: {e}");
            return;
        }
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            println!("[Device] failed to create {}: {e}", parent.display());
            return;
        }
    }
    if let Err(e) = std::fs::write(&path, json) {
        println!(
            "[Device] failed to write {}: {e} - identity will not persist",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_mac_uses_a_real_oui() {
        let id = DeviceIdentity::generate();
        let prefix: String = id.mac.chars().take(8).collect();
        assert!(OUIS.contains(&prefix.as_str()), "unexpected OUI: {}", id.mac);
        assert_eq!(id.mac.len(), 17);
    }

    #[test]
    fn a_key_keeps_the_same_identity_once_stored() {
        let mut store = BTreeMap::new();
        let (first, dirty) = resolve(&mut store, "acct", None);
        assert!(dirty, "a new key must be written back");

        let (second, dirty) = resolve(&mut store, "acct", None);
        assert!(!dirty, "an existing key must not rewrite the file");
        assert_eq!(first.rid, second.rid);
        assert_eq!(first.mac, second.mac);
        assert_eq!(first.wk, second.wk);
        assert_eq!(first.hash2_seed, second.hash2_seed);
    }

    #[test]
    fn different_keys_get_different_identities() {
        let mut store = BTreeMap::new();
        let (a, _) = resolve(&mut store, "acct_a", None);
        let (b, _) = resolve(&mut store, "acct_b", None);
        assert_ne!(a.rid, b.rid);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn ltoken_values_win_but_the_hash2_seed_survives() {
        let mut store = BTreeMap::new();
        let (first, _) = resolve(&mut store, "RID1", None);

        let (second, dirty) = resolve(
            &mut store,
            "RID1",
            Some(("RID1".into(), "AA:BB:CC:DD:EE:FF".into(), "W".repeat(32))),
        );
        assert!(dirty);
        assert_eq!(second.mac, "AA:BB:CC:DD:EE:FF");
        assert_eq!(second.hash2_seed, first.hash2_seed);

        let (third, dirty) = resolve(
            &mut store,
            "RID1",
            Some(("RID1".into(), "AA:BB:CC:DD:EE:FF".into(), "W".repeat(32))),
        );
        assert!(!dirty, "unchanged values must not rewrite the file");
        assert_eq!(third.mac, second.mac);
    }

    #[test]
    fn generated_fields_have_the_lengths_the_server_expects() {
        let id = DeviceIdentity::generate();
        assert_eq!(id.rid.len(), 32);
        assert_eq!(id.wk.len(), 32);
        assert_eq!(id.hash2_seed.len(), 16);
    }

    #[test]
    fn a_stored_playtime_survives_a_reload() {
        let mut store = BTreeMap::new();
        let (_, _) = resolve(&mut store, "acct", None);
        store.get_mut("acct").unwrap().total_playtime = 4242;

        let json = serde_json::to_string(&store).unwrap();
        let reloaded: BTreeMap<String, DeviceIdentity> = serde_json::from_str(&json).unwrap();
        assert_eq!(reloaded["acct"].total_playtime, 4242);
    }

    #[test]
    fn generated_player_age_is_plausible() {
        for _ in 0..100 {
            let age = DeviceIdentity::generate().player_age;
            assert!((18..=35).contains(&age), "implausible age: {age}");
        }
    }

    #[test]
    fn a_store_written_before_personas_existed_still_loads() {
        let json = r#"{
            "acct": {
                "rid": "0123456789ABCDEF0123456789ABCDEF",
                "mac": "A4:83:E7:11:22:33",
                "wk": "FEDCBA9876543210FEDCBA9876543210",
                "hash2_seed": "0123456789ABCDEF"
            }
        }"#;
        let store: BTreeMap<String, DeviceIdentity> = serde_json::from_str(json).unwrap();
        let id = &store["acct"];
        assert_eq!(id.mac, "A4:83:E7:11:22:33");
        assert_eq!(id.country, "us");
        assert_eq!(id.player_age, 20);
        assert_eq!(id.cbits, 1024);
    }
}
