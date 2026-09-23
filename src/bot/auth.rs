use crate::constants::{GAME_VER, PROTOCOL};
use crate::dashboard::get_dashboard_proxied;
use crate::device::DeviceIdentity;
use crate::login::{LoginError, get_legacy_token_proxied};
use crate::server_data::{LoginInfo, get_server_data_proxied};
use std::net::SocketAddr;

use super::shared::Socks5Config;

pub(super) struct Credentials {
    pub ltoken: String,
    pub meta: String,
    pub addr: SocketAddr,
}

/// How many times a step of the login chain is retried before the bot gives up.
/// Retrying forever hides a permanent problem — an IP the login host refuses, a
/// dead proxy — behind a log line every five seconds.
const MAX_ATTEMPTS: u32 = 6;
const FIRST_BACKOFF_SECS: u64 = 5;
const MAX_BACKOFF_SECS: u64 = 60;

/// Backoff for attempt `n` (1-based): 5s, 10s, 20s, 40s, 60s, 60s...
fn backoff_secs(attempt: u32) -> u64 {
    (FIRST_BACKOFF_SECS << attempt.saturating_sub(1).min(8)).min(MAX_BACKOFF_SECS)
}

pub(super) fn fetch_credentials(
    username: &str,
    password: &str,
    proxy: Option<&Socks5Config>,
    device: &DeviceIdentity,
    log: &mut dyn FnMut(String),
) -> Result<Credentials, String> {
    let proxy_url = proxy.map(|p| p.to_url());
    let proxy_url = proxy_url.as_deref();

    let login_info = LoginInfo {
        protocol: PROTOCOL,
        game_version: GAME_VER.into(),
    };

    let mut alternate = false;
    let mut attempt = 0;
    loop {
        attempt += 1;
        if attempt > MAX_ATTEMPTS {
            return Err(format!("login failed after {MAX_ATTEMPTS} attempts"));
        }

        let mut retry = |log: &mut dyn FnMut(String), reason: String| {
            if attempt >= MAX_ATTEMPTS {
                // Last attempt: the caller gives up on the next pass, so do not sleep.
                log(format!(
                    "[Bot] fetch: {reason} - attempt {attempt}/{MAX_ATTEMPTS}, giving up"
                ));
                return;
            }
            let wait = backoff_secs(attempt);
            log(format!(
                "[Bot] fetch: {reason} - attempt {attempt}/{MAX_ATTEMPTS}, retrying in {wait}s"
            ));
            std::thread::sleep(std::time::Duration::from_secs(wait));
        };

        log(format!(
            "[Bot] fetching server_data (alternate={alternate})..."
        ));
        let server_data = match get_server_data_proxied(alternate, &login_info, proxy_url) {
            Ok(s) => s,
            Err(e) => {
                alternate = !alternate;
                retry(log, format!("server_data failed: {e}"));
                continue;
            }
        };

        let dashboard = match get_dashboard_proxied(
            &server_data.loginurl,
            &login_info,
            &server_data.meta,
            proxy_url,
            device,
        ) {
            Ok(d) => d,
            Err(e) => {
                retry(log, format!("dashboard failed: {e}"));
                continue;
            }
        };

        let growtopia_url = match dashboard.growtopia {
            Some(u) => u,
            None => {
                retry(log, "no Growtopia URL in dashboard".to_string());
                continue;
            }
        };

        let ltoken = match get_legacy_token_proxied(&growtopia_url, username, password, proxy_url) {
            Ok(t) => t,
            Err(e) => {
                // Credential problems never fix themselves by waiting.
                if matches!(e, LoginError::Exhausted | LoginError::WrongCredentials) {
                    return Err(e.to_string());
                }
                retry(log, format!("login failed: {e}"));
                continue;
            }
        };

        let addr: SocketAddr = match format!("{}:{}", server_data.server, server_data.port).parse() {
            Ok(a) => a,
            Err(e) => return Err(format!("server sent an unusable address: {e}")),
        };

        log("[Bot] Got token".to_string());
        return Ok(Credentials {
            ltoken,
            meta: server_data.meta,
            addr,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{backoff_secs, MAX_BACKOFF_SECS};

    #[test]
    fn backoff_grows_then_settles_at_the_cap() {
        assert_eq!(backoff_secs(1), 5);
        assert_eq!(backoff_secs(2), 10);
        assert_eq!(backoff_secs(3), 20);
        assert_eq!(backoff_secs(4), 40);
        assert_eq!(backoff_secs(5), MAX_BACKOFF_SECS);
        assert_eq!(backoff_secs(50), MAX_BACKOFF_SECS);
    }
}
