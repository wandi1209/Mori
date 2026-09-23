use axum::{
    Router,
    body::Body,
    extract::{Path, Query, Request, State, WebSocketUpgrade},
    extract::ws::{Message, WebSocket},
    http::{StatusCode, Method},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::net::{ToSocketAddrs, SocketAddr};

use crate::auth::AuthState;
use crate::bot::Socks5Config;
use crate::bot_manager::{BotInfo, BotManager};
use crate::bot_state::{ActiveHours, BotCommand, BotDelays, BotState};
use crate::events::WsTx;
use crate::items::ItemInfo;
use crate::proxy_test::{ProxyTestResult, run_proxy_test};

pub type SharedManager = Arc<Mutex<BotManager>>;

#[derive(Clone)]
pub struct AppState {
    pub manager: SharedManager,
    pub ws_tx:   WsTx,
    pub auth:    AuthState,
}

// ── Auth middleware ────────────────────────────────────────────────────────────

/// Extract the Bearer token from the `Authorization` header.
fn extract_bearer(req: &Request) -> Option<String> {
    let hdr = req.headers().get("Authorization")?.to_str().ok()?;
    hdr.strip_prefix("Bearer ").map(str::to_owned)
}

async fn auth_middleware(
    State(s): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    if path.starts_with("/auth/")
        || path == "/"
        || path.starts_with("/assets/")
        || path.starts_with("/growtopia-cdn/")
    {
        return next.run(req).await;
    }

    // WebSocket token is passed as a query param `?token=…`
    if path == "/ws" {
        let token = req.uri().query()
            .and_then(|q| {
                url::form_urlencoded::parse(q.as_bytes())
                    .find(|(k, _)| k == "token")
                    .map(|(_, v)| v.into_owned())
            });
        if let Some(t) = token {
            if s.auth.validate_token(&t) {
                return next.run(req).await;
            }
        }
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // All other routes need a valid Bearer token.
    if let Some(token) = extract_bearer(&req) {
        if s.auth.validate_token(&token) {
            return next.run(req).await;
        }
    }
    StatusCode::UNAUTHORIZED.into_response()
}

// ── Auth handlers ─────────────────────────────────────────────────────────────

/// GET /auth/status  →  { registered: bool }
async fn auth_status(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "registered": s.auth.is_registered() }))
}

#[derive(Deserialize)]
struct SetupRequest {
    password: String,
}

/// POST /auth/setup  →  registers the single user (only works once)
async fn auth_setup(
    State(s): State<AppState>,
    Json(req): Json<SetupRequest>,
) -> Response {
    if s.auth.is_registered() {
        return (StatusCode::CONFLICT, Json(serde_json::json!({ "error": "already registered" }))).into_response();
    }
    match s.auth.register(&req.password) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ).into_response(),
    }
}

#[derive(Deserialize)]
struct LoginRequest {
    password: String,
}

/// POST /auth/login  →  { token: "…" }
async fn auth_login(
    State(s): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Response {
    match s.auth.login(&req.password) {
        Some(token) => Json(serde_json::json!({ "token": token })).into_response(),
        None => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "invalid password" })),
        ).into_response(),
    }
}

/// POST /auth/logout
async fn auth_logout(State(s): State<AppState>) -> StatusCode {
    s.auth.logout();
    StatusCode::NO_CONTENT
}

// ── Bot handlers ───────────────────────────────────────────────────────────────

async fn list_bots(State(s): State<AppState>) -> Json<Vec<BotInfo>> {
    Json(s.manager.lock().unwrap().list())
}

#[derive(Deserialize)]
struct SpawnRequest {
    username:       String,
    password:       String,
    proxy_host:     Option<String>,
    proxy_port:     Option<u16>,
    proxy_username: Option<String>,
    proxy_password: Option<String>,
}

async fn spawn_bot(
    State(s): State<AppState>,
    Json(req): Json<SpawnRequest>,
) -> Json<serde_json::Value> {
    let proxy = match (req.proxy_host, req.proxy_port) {
        (Some(host), Some(port)) => {
            let addr = format!("{}:{}", host, port)
            .parse()
            .or_else(|_| {
                // Try to resolve the host if it's not a valid socket address
                let mut addrs = format!("{}:{}", host, port)
                    .to_socket_addrs()
                    .map_err(|_| StatusCode::BAD_REQUEST)?;
                addrs.next().ok_or(StatusCode::BAD_REQUEST)
            })
            .ok();
            addr.map(|proxy_addr| Socks5Config {
                proxy_addr,
                username: req.proxy_username,
                password: req.proxy_password,
            })
        }
        _ => None,
    };
    let id = s.manager.lock().unwrap().spawn(req.username, req.password, proxy);
    Json(serde_json::json!({ "id": id }))
}

#[derive(Deserialize)]
struct SpawnLtokenRequest {
    ltoken:         String,
    proxy_host:     Option<String>,
    proxy_port:     Option<u16>,
    proxy_username: Option<String>,
    proxy_password: Option<String>,
}

async fn spawn_ltoken_bot(
    State(s): State<AppState>,
    Json(req): Json<SpawnLtokenRequest>,
) -> Json<serde_json::Value> {
    let proxy = match (req.proxy_host, req.proxy_port) {
        (Some(host), Some(port)) => {
            let addr = format!("{}:{}", host, port).parse().ok();
            addr.map(|proxy_addr| Socks5Config {
                proxy_addr,
                username: req.proxy_username,
                password: req.proxy_password,
            })
        }
        _ => None,
    };
    let id = s.manager.lock().unwrap().spawn_ltoken(req.ltoken, proxy);
    Json(serde_json::json!({ "id": id }))
}

#[derive(Deserialize)]
struct GoogleUrlRequest {
    /// Label the account's device identity is stored under in data/devices.json.
    account:        String,
    proxy_host:     Option<String>,
    proxy_port:     Option<u16>,
    proxy_username: Option<String>,
    proxy_password: Option<String>,
}

#[derive(Serialize)]
struct GoogleUrlResponse {
    url: String,
}

/// Account labels a sign-in link has been issued for since start-up. A token is
/// bound to the device values of the dashboard request that produced it, so
/// spawning under a different label presents the token with the wrong machine and
/// the game server drops the connection without a word.
static GOOGLE_LINKS_ISSUED: std::sync::Mutex<Option<std::collections::HashSet<String>>> =
    std::sync::Mutex::new(None);

fn remember_google_link(account: &str) {
    let mut guard = GOOGLE_LINKS_ISSUED.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(std::collections::HashSet::new)
        .insert(account.to_string());
}

fn google_link_was_issued_for(account: &str) -> bool {
    let guard = GOOGLE_LINKS_ISSUED.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().is_some_and(|set| set.contains(account))
}

/// Returns the Google sign-in URL for an account, to be opened in a real browser.
///
/// The request that produces it has to carry the same device values the bot will
/// log in with, so it goes out with the identity stored for `account` — the same
/// one `POST /bots/google` will pair the returned token with.
async fn google_login_url(
    Json(req): Json<GoogleUrlRequest>,
) -> Result<Json<GoogleUrlResponse>, (StatusCode, String)> {
    if req.account.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "account is required".into()));
    }

    let proxy = socks5_from_parts(
        req.proxy_host,
        req.proxy_port,
        req.proxy_username,
        req.proxy_password,
    );
    let account = req.account.trim().to_string();
    let label_for_memo = account.clone();

    let links = tokio::task::spawn_blocking(move || {
        println!("[Google] fetching sign-in link for {account}");
        let device = crate::device::load_or_create(&account);
        let login_info = crate::server_data::LoginInfo {
            protocol: crate::constants::protocol(),
            game_version: crate::constants::game_version().into(),
        };
        let proxy_url = proxy.as_ref().map(|p: &Socks5Config| p.to_url());
        let server_data = crate::server_data::get_server_data_proxied(
            false,
            &login_info,
            proxy_url.as_deref(),
        )
        .map_err(|e| format!("server_data failed: {e}"))?;

        crate::dashboard::get_dashboard_proxied(
            &server_data.loginurl,
            &login_info,
            &server_data.meta,
            proxy_url.as_deref(),
            &device,
        )
        .map_err(|e| format!("dashboard failed: {e}"))
    })
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "task failed".to_string()))?
    .inspect_err(|e| println!("[Google] {e}"))
    .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;

    match links.google {
        Some(url) => {
            println!("[Google] sign-in link ready");
            remember_google_link(&label_for_memo);
            Ok(Json(GoogleUrlResponse { url }))
        }
        None => {
            let reason = "dashboard did not offer a Google option";
            println!("[Google] {reason}");
            Err((StatusCode::BAD_GATEWAY, reason.into()))
        }
    }
}

#[derive(Deserialize)]
struct SpawnGoogleRequest {
    account:        String,
    token:          String,
    proxy_host:     Option<String>,
    proxy_port:     Option<u16>,
    proxy_username: Option<String>,
    proxy_password: Option<String>,
}

/// Pulls the login token out of whatever the browser left the user holding.
///
/// A Google sign-in ends on a page showing the validation response itself —
/// `{"status":"success","message":"Account Validated.","token":"...", ...}` — so
/// the whole thing can be pasted. A URL carrying `token=`, including the
/// `growtopia://` deep link, works too, as does the bare token.
fn extract_token(input: &str) -> String {
    let input = input.trim();

    if input.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(input) {
            if let Some(token) = v["token"].as_str() {
                return token.to_string();
            }
        }
    }

    if let Some((_, rest)) = input.split_once("token=") {
        let value = rest.split(['&', '#', '"']).next().unwrap_or(rest);
        return urlencoding::decode(value)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| value.to_string());
    }

    // A token copied out of JSON by hand keeps its escaped slashes.
    input.replace("\\/", "/")
}

/// Spawns a bot from a token obtained through the browser sign-in above.
async fn spawn_google_bot(
    State(s): State<AppState>,
    Json(req): Json<SpawnGoogleRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let account = req.account.trim().to_string();
    let token = extract_token(&req.token);
    if account.is_empty() || token.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "account and token are required".into(),
        ));
    }

    if !google_link_was_issued_for(&account) {
        println!(
            "[Google] warning: no sign-in link was issued for {account:?} in this run. \
             A token only works with the device values of the request that produced it, \
             so make sure this is the same label you fetched the link with."
        );
    }

    let proxy = socks5_from_parts(
        req.proxy_host,
        req.proxy_port,
        req.proxy_username,
        req.proxy_password,
    );
    let id = s.manager.lock().unwrap().spawn_token(account, token, proxy);
    Ok(Json(serde_json::json!({ "id": id })))
}

/// Builds a SOCKS5 config from the proxy fields every spawn endpoint accepts.
/// Host and port are required together; a host that is not already an address is
/// resolved.
fn socks5_from_parts(
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    password: Option<String>,
) -> Option<Socks5Config> {
    let (host, port) = match (host, port) {
        (Some(h), Some(p)) => (h, p),
        _ => return None,
    };

    let addr = format!("{host}:{port}")
        .parse()
        .ok()
        .or_else(|| format!("{host}:{port}").to_socket_addrs().ok()?.next())?;

    Some(Socks5Config {
        proxy_addr: addr,
        username,
        password,
    })
}

async fn stop_bot(
    State(s): State<AppState>,
    Path(id): Path<u32>,
) -> StatusCode {
    if s.manager.lock().unwrap().stop(id) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn bot_state(
    State(s): State<AppState>,
    Path(id): Path<u32>,
) -> Result<Json<BotState>, StatusCode> {
    s.manager.lock().unwrap()
        .get_state(id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CmdRequest {
    Move { x: i32, y: i32 },
    WalkTo { x: u32, y: u32 },
    RunScript { content: String },
    StopScript,
    Wear { item_id: u32 },
    Unwear { item_id: u32 },
    Drop { item_id: u32, count: u32 },
    Trash { item_id: u32, count: u32 },
    SetDelays(BotDelays),
    SetActiveHours(ActiveHours),
    SetAutoCollect { enabled: bool },
    SetCollectConfig {
        radius_tiles: u8,
        #[serde(default)]
        blacklist: Vec<u16>,
    },
    SetAutoReconnect { enabled: bool },
    Disconnect,
    Reconnect,
    AcceptAccess,
    Warp { name: String, id: String },
}

#[derive(Deserialize)]
struct ItemsQuery {
    page:      Option<usize>,
    q:         Option<String>,
    #[serde(rename = "get-items")]
    get_items: Option<String>,
}

#[derive(serde::Serialize)]
struct ItemsResponse {
    items:     Vec<ItemInfo>,
    total:     usize,
    page:      usize,
    page_size: usize,
}

const ITEMS_PAGE_SIZE: usize = 50;

async fn list_items(
    State(s): State<AppState>,
    Query(params): Query<ItemsQuery>,
) -> axum::response::Response {
    let mgr = s.manager.lock().unwrap();

    if let Some(ids_str) = params.get_items {
        let ids: std::collections::HashSet<u32> = ids_str
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        let items: Vec<ItemInfo> = mgr.items_dat.items.iter()
            .filter(|i| ids.contains(&i.id))
            .cloned()
            .collect();
        return Json(items).into_response();
    }

    let q = params.q.as_deref().unwrap_or("").to_lowercase();
    let page = params.page.unwrap_or(1).max(1);

    let filtered: Vec<&ItemInfo> = mgr.items_dat.items.iter().filter(|i| {
        if q.is_empty() { return true; }
        if let Ok(id) = q.parse::<u32>() { if i.id == id { return true; } }
        i.name.to_lowercase().contains(&q)
    }).collect();

    let total = filtered.len();
    let start = (page - 1) * ITEMS_PAGE_SIZE;
    let items = filtered.into_iter().skip(start).take(ITEMS_PAGE_SIZE).cloned().collect();

    Json(ItemsResponse { items, total, page, page_size: ITEMS_PAGE_SIZE }).into_response()
}

async fn item_names(State(s): State<AppState>) -> Json<std::collections::HashMap<u32, String>> {
    let mgr = s.manager.lock().unwrap();
    let map = mgr.items_dat.items.iter()
        .map(|i| (i.id, i.name.clone()))
        .collect();
    Json(map)
}

async fn item_colors(State(s): State<AppState>) -> Json<std::collections::HashMap<u32, u32>> {
    let mgr = s.manager.lock().unwrap();
    let map = mgr.items_dat.items.iter()
        .map(|i| {
            let raw = if i.id % 2 == 0 {
                // Block: use seed (id+1) base_color
                mgr.items_dat.find_by_id(i.id + 1)
                    .map(|seed| seed.base_color)
                    .unwrap_or(i.base_color)
            } else {
                // Seed: use own overlay_color
                i.overlay_color
            };
            (i.id, crate::items::bgra_to_rgb(raw))
        })
        .collect();
    Json(map)
}

async fn bot_cmd(
    State(s): State<AppState>,
    Path(id): Path<u32>,
    Json(req): Json<CmdRequest>,
) -> StatusCode {
    let cmd = match req {
        CmdRequest::Move { x, y }   => BotCommand::Move { x, y },
        CmdRequest::WalkTo { x, y } => BotCommand::WalkTo { x, y },
        CmdRequest::RunScript { content }   => BotCommand::RunScript { content },
        CmdRequest::StopScript              => BotCommand::StopScript,
        CmdRequest::Wear { item_id }        => BotCommand::Wear { item_id },
        CmdRequest::Unwear { item_id }      => BotCommand::Unwear { item_id },
        CmdRequest::Drop { item_id, count } => BotCommand::Drop { item_id, count },
        CmdRequest::Trash { item_id, count } => BotCommand::Trash { item_id, count },
        CmdRequest::SetDelays(d) => BotCommand::SetDelays(d),
        CmdRequest::SetActiveHours(c) => BotCommand::SetActiveHours(c),
        CmdRequest::SetAutoCollect { enabled } => BotCommand::SetAutoCollect { enabled },
        CmdRequest::SetCollectConfig {
            radius_tiles,
            blacklist,
        } => BotCommand::SetCollectConfig {
            radius_tiles,
            blacklist,
        },
        CmdRequest::SetAutoReconnect { enabled } => BotCommand::SetAutoReconnect { enabled },
        CmdRequest::Disconnect => BotCommand::Disconnect,
        CmdRequest::Reconnect => BotCommand::Reconnect,
        CmdRequest::AcceptAccess => BotCommand::AcceptAccess,
        CmdRequest::Warp { name, id } => BotCommand::Warp { name, id },
    };
    if s.manager.lock().unwrap().send_cmd(id, cmd) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ── Proxy test ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ProxyTestRequest {
    proxy_host:     String,
    proxy_port:     u16,
    proxy_username: Option<String>,
    proxy_password: Option<String>,
}

async fn proxy_check(
    Json(req): Json<ProxyTestRequest>,
) -> Result<Json<ProxyTestResult>, StatusCode> {
    let addr = format!("{}:{}", req.proxy_host, req.proxy_port)
        .parse()
        .or_else(|_| {
            // Try to resolve the host if it's not a valid socket address
            let mut addrs = format!("{}:{}", req.proxy_host, req.proxy_port)
                .to_socket_addrs()
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            addrs.next().ok_or(StatusCode::BAD_REQUEST)
        })?;

    let cfg = Socks5Config {
        proxy_addr: addr,
        username: req.proxy_username,
        password: req.proxy_password,
    };

    let result = tokio::task::spawn_blocking(move || run_proxy_test(cfg))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(result))
}

// ── WebSocket handler ─────────────────────────────────────────────────────────

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(s): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, s.ws_tx.subscribe()))
}

async fn handle_socket(
    mut socket: WebSocket,
    mut rx: tokio::sync::broadcast::Receiver<crate::events::WsEvent>,
) {
    loop {
        match rx.recv().await {
            Ok(event) => {
                let msg = match serde_json::to_string(&event) {
                    Ok(s)  => s,
                    Err(_) => continue,
                };
                if socket.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
}

async fn growtopia_cdn(Path(path): Path<String>) -> Response {
    let url = format!("https://growserver-cache.netlify.app/{}", path);
    match tokio::task::spawn_blocking(move || ureq::get(&url).call()).await {
        Ok(Ok(resp)) => {
            let content_type = resp.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_owned();
            let bytes = match resp.into_body().read_to_vec() {
                Ok(b) => b,
                Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
            };
            Response::builder()
                .header("content-type", content_type)
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Ok(Err(_)) | Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}

async fn index_html() -> impl IntoResponse {
    let dist = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("dist");

    let html = tokio::fs::read_to_string(dist.join("index.html"))
        .await
        .unwrap_or_default();

    let injected = html.replace(
        "</body>",
        r#"<div style="position:fixed;bottom:8px;right:12px;font-size:10px;opacity:0.35;color:#fff;pointer-events:none;z-index:9999;font-family:sans-serif;">Mori created with &#x2764;&#xfe0e; by Cendy</div></body>"#,
    );

    axum::response::Html(injected)
}

pub async fn serve(manager: SharedManager, ws_tx: WsTx) {
    let auth = AuthState::new();
    let state = AppState { manager, ws_tx, auth };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers(Any);

    let dist = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("dist");

    let app = Router::new()
        .route("/", get(index_html))

        // Auth endpoints (public)
        .route("/auth/status", get(auth_status))
        .route("/auth/setup",  post(auth_setup))
        .route("/auth/login",  post(auth_login))
        .route("/auth/logout", post(auth_logout))

        // Protected API
        .route("/bots", get(list_bots).post(spawn_bot))
        .route("/bots/ltoken", post(spawn_ltoken_bot))
        .route("/bots/google", post(spawn_google_bot))
        .route("/bots/google/url", post(google_login_url))
        .route("/bots/{id}", delete(stop_bot))
        .route("/bots/{id}/state", get(bot_state))
        .route("/bots/{id}/cmd", post(bot_cmd))
        .route("/items", get(list_items))
        .route("/items/names", get(item_names))
        .route("/items/colors", get(item_colors))
        .route("/proxy/test", post(proxy_check))
        .route("/growtopia-cdn/{*path}", get(growtopia_cdn))
        .route("/ws", get(ws_handler))

        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware))
        .layer(cors)
        .with_state(state)
        .fallback_service(ServeDir::new(&dist).fallback(ServeFile::new(dist.join("index.html"))));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("Dashboard  http://localhost:3000");
    println!("WebSocket  ws://localhost:3000/ws");
    println!("API        http://localhost:3000/bots");
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::extract_token;

    #[test]
    fn a_bare_token_is_left_alone() {
        assert_eq!(extract_token("  ABC123  "), "ABC123");
    }

    #[test]
    fn a_token_is_lifted_out_of_a_url() {
        assert_eq!(
            extract_token("https://login.growtopiagame.com/player/login/dashboard?token=ABC123"),
            "ABC123"
        );
    }

    #[test]
    fn a_token_is_lifted_out_of_the_deep_link() {
        assert_eq!(extract_token("growtopia://login?token=ABC123&foo=1"), "ABC123");
    }

    #[test]
    fn percent_escapes_are_decoded() {
        assert_eq!(extract_token("https://x/?token=a%2Fb%2Bc%3D"), "a/b+c=");
    }

    #[test]
    fn the_whole_validation_json_can_be_pasted() {
        let page = r#"{"status":"success","message":"Account Validated.","token":"AbC\/dEf+gh==","url":"","accountType":"google","accountAge":2}"#;
        assert_eq!(extract_token(page), "AbC/dEf+gh==");
    }

    #[test]
    fn slashes_escaped_by_hand_copying_are_restored() {
        assert_eq!(extract_token(r"AbC\/dEf+gh=="), "AbC/dEf+gh==");
    }
}
