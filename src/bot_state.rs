use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::mpsc;
use crate::world::TileType;

#[derive(Default, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BotStatus {
    #[default]
    Connecting,
    Connected,
    InGame,
    /// Blocked by 2FA (Advanced Account Protection). Retrying after 120 s.
    TwoFactorAuth,
    /// Server overloaded. Retrying after 30 s.
    ServerOverloaded,
    /// Too many logins at once. Retrying after 5 s.
    TooManyLogins,
    /// Client is outdated — server requires an update. Bot stopped.
    UpdateRequired,
    /// Server is under maintenance. Retrying after 600 s.
    Maintenance,
    /// Logged out on purpose: outside the configured active hours, or on a break
    /// between sessions. `BotState::status_detail` says when it comes back.
    Resting,
    /// The HTTP login chain gave up. `BotState::status_detail` says why. Bot stopped.
    LoginFailed,
}

impl fmt::Display for BotStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BotStatus::Connecting    => write!(f, "connecting"),
            BotStatus::Connected     => write!(f, "connected"),
            BotStatus::InGame        => write!(f, "in_game"),
            BotStatus::TwoFactorAuth    => write!(f, "two_factor_auth"),
            BotStatus::ServerOverloaded => write!(f, "server_overloaded"),
            BotStatus::TooManyLogins    => write!(f, "too_many_logins"),
            BotStatus::UpdateRequired   => write!(f, "update_required"),
            BotStatus::Maintenance      => write!(f, "maintenance"),
            BotStatus::LoginFailed      => write!(f, "login_failed"),
            BotStatus::Resting          => write!(f, "resting"),
        }
    }
}

#[derive(Clone, Serialize)]
pub struct TileInfo {
    pub fg_item_id: u16,
    pub bg_item_id: u16,
    pub flags:      u16,
    pub tile_type:  TileType,
}

impl Default for TileInfo {
    fn default() -> Self {
        Self { fg_item_id: 0, bg_item_id: 0, flags: 0, tile_type: TileType::Basic }
    }
}

#[derive(Clone, Serialize)]
pub struct PlayerInfo {
    pub net_id:  u32,
    pub name:    String,
    pub pos_x:   f32,
    pub pos_y:   f32,
    pub country: String,
}

#[derive(Clone, Serialize)]
pub struct InvSlot {
    pub item_id:     u16,
    pub amount:      u8,
    pub is_active:   bool,
    pub action_type: u8,
}

#[derive(Clone, Serialize)]
pub struct WorldObjectInfo {
    pub uid:     u32,
    pub item_id: u16,
    pub x:       f32,
    pub y:       f32,
    pub count:   u8,
}

#[derive(Default, Clone, Serialize)]
pub struct TrackInfo {
    pub level:           u32,
    pub grow_id:         u64,
    pub install_date:    u64,
    pub global_playtime: u64,
    pub awesomeness:     u32,
}

/// When a bot is allowed to be online.
///
/// A bot that is connected every hour of every day, without a single break, is
/// distinguishable from a player on playtime alone — no behavioural analysis
/// needed. Within the daily window the bot also alternates play sessions with
/// breaks, both randomised by `jitter_pct`.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct ActiveHours {
    pub enabled: bool,
    /// Start of the daily window, minutes after local midnight.
    pub start_minute: u16,
    /// End of the window, minutes after local midnight. Equal to `start_minute`
    /// means the whole day; a value below it means the window crosses midnight.
    pub end_minute: u16,
    /// Length of one play session in minutes. 0 disables breaks entirely.
    pub session_minutes: u32,
    /// Length of a break between sessions, in minutes.
    pub break_minutes: u32,
    /// Random spread applied to both lengths, in percent (0-90).
    pub jitter_pct: u8,
}

impl Default for ActiveHours {
    fn default() -> Self {
        Self {
            enabled: false,
            start_minute: 8 * 60,
            end_minute: 23 * 60,
            session_minutes: 90,
            break_minutes: 20,
            jitter_pct: 30,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BotDelays {
    pub place_ms:             u64,
    pub walk_ms:              u64,
    /// Random jitter applied to `place_ms`/`walk_ms`, in percent (0-90).
    /// Constant delays are trivially recognisable as automation.
    #[serde(default = "default_jitter_pct")]
    pub jitter_pct:           u8,
    pub twofa_secs:           u64,
    pub server_overload_secs: u64,
    pub too_many_logins_secs: u64,
    pub maintenance_secs:     u64,
}

fn default_jitter_pct() -> u8 {
    25
}

impl Default for BotDelays {
    fn default() -> Self {
        Self {
            place_ms:             500,
            walk_ms:              500,
            jitter_pct:           default_jitter_pct(),
            twofa_secs:           120,
            server_overload_secs: 30,
            too_many_logins_secs: 5,
            maintenance_secs:     600,
        }
    }
}

#[derive(Clone, Serialize)]
pub struct BotState {
    pub status:          BotStatus,
    /// Why the bot is in its current status, when there is something to say —
    /// currently the reason a login was abandoned.
    pub status_detail:   Option<String>,
    pub username:        String,
    pub mac:             String,
    pub world_name:      String,
    /// Tile-coordinate position (pixels ÷ 32).
    pub pos_x:           f32,
    pub pos_y:           f32,
    pub world_width:     u32,
    pub world_height:    u32,
    pub tiles:           Vec<TileInfo>,
    pub objects:         Vec<WorldObjectInfo>,
    pub players:         Vec<PlayerInfo>,
    pub inventory:       Vec<InvSlot>,
    /// Maximum number of inventory slots the bot has (from SendInventoryState).
    pub inventory_size:  u32,
    pub gems:            i32,
    pub console:         Vec<String>,
    /// Round-trip time in milliseconds from ENet, updated every run loop tick.
    pub ping_ms: u32,
    /// Configurable delays for bot actions.
    pub delays: BotDelays,
    /// When this bot is allowed to be online.
    pub active_hours: ActiveHours,
    /// Whether a Lua script is running on this bot right now. Tracked from the
    /// script thread's channel, so it turns itself off when a script ends or
    /// crashes rather than staying on until someone presses Stop.
    pub script_running: bool,
    pub track_info: Option<TrackInfo>,
    /// Whether the run loop should auto-collect nearby dropped items.
    pub auto_collect: bool,
    /// Auto-collect half-extent in tiles (1–5): axis-aligned square |Δx|,|Δy| ≤ tiles×32 px.
    /// Auto-collect radius in tiles (1-5). A client collects what the character
    /// touches, so 1 is the honest value; drops scatter a couple of tiles when a
    /// tree is harvested, and gems further still, so the default trades some of
    /// that for not leaving the crop on the floor.
    pub collect_radius_tiles: u8,
    /// Item IDs to skip when auto-collecting (sorted, unique in API responses).
    pub collect_blacklist: Vec<u16>,
    /// Skip gems (item ID 112) during auto-collect when true.
    pub ignore_gems: bool,
    /// Skip essences (item IDs 5024/5026/5028/5030) during auto-collect when true.
    pub ignore_essences: bool,
    /// Leave world automatically when a mod is detected via OnSpawn.
    /// Leave the world when a mod (or an invisible mod) spawns in it. On by
    /// default: a moderator watching the character move is the likeliest way a
    /// bot is noticed at all.
    pub auto_leave_on_mod: bool,
    /// Send `/ban <name>` when any non-local player spawns.
    pub auto_ban: bool,
    /// Skip objects with no reachable A* path during auto-collect.
    pub collect_path_check: bool,
    /// Whether the bot should automatically reconnect after a disconnect.
    pub auto_reconnect: bool,
}

impl Default for BotState {
    fn default() -> Self {
        Self {
            status: BotStatus::default(),
            status_detail: None,
            username: String::new(),
            mac: String::new(),
            world_name: String::new(),
            pos_x: 0.0,
            pos_y: 0.0,
            world_width: 0,
            world_height: 0,
            tiles: Vec::new(),
            objects: Vec::new(),
            players: Vec::new(),
            inventory: Vec::new(),
            inventory_size: 0,
            gems: 0,
            console: Vec::new(),
            ping_ms: 0,
            delays: BotDelays::default(),
            active_hours: ActiveHours::default(),
            script_running: false,
            track_info: None,
            auto_collect: true,
            collect_radius_tiles: 3,
            collect_blacklist: Vec::new(),
            ignore_gems: false,
            ignore_essences: false,
            auto_leave_on_mod: true,
            auto_ban: false,
            collect_path_check: true,
            auto_reconnect: true,
        }
    }
}

pub enum BotCommand {
    Move { x: i32, y: i32 },
    WalkTo { x: u32, y: u32 },
    RunScript { content: String },
    StopScript,
    Say { text: String },
    Warp { name: String, id: String },
    Disconnect,
    Reconnect,
    Place { x: i32, y: i32, item: u32 },
    Hit { x: i32, y: i32 },
    Wrench { x: i32, y: i32 },
    Wear { item_id: u32 },
    Unwear { item_id: u32 },
    Drop { item_id: u32, count: u32 },
    Trash { item_id: u32, count: u32 },
    LeaveWorld,
    Respawn,
    FindPath { x: u32, y: u32 },
    SetDelays(BotDelays),
    SetActiveHours(ActiveHours),
    SetAutoCollect { enabled: bool },
    SetCollectConfig {
        radius_tiles: u8,
        blacklist: Vec<u16>,
    },
    SetAutoReconnect { enabled: bool },
    AcceptAccess,
}

pub type CmdSender   = mpsc::Sender<BotCommand>;
pub type CmdReceiver = mpsc::Receiver<BotCommand>;
