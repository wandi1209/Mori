use std::time::Duration;
use scraper::{Html, Selector};
use crate::constants::fhash;
use crate::device::DeviceIdentity;
use crate::protocol::crypto::{compute_klv, hash_string};
use crate::server_data::LoginInfo;
use serde_json;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;


#[derive(Debug)]
pub struct DashboardLinks {
    pub apple: Option<String>,
    pub google: Option<String>,
    pub growtopia: Option<String>,
}

pub fn get_dashboard(
    login_url: &str,
    login_info: &LoginInfo,
    meta: &str,
    device: &DeviceIdentity,
) -> Result<DashboardLinks> {
    get_dashboard_proxied(login_url, login_info, meta, None, device)
}

pub fn get_dashboard_proxied(
    login_url: &str,
    login_info: &LoginInfo,
    meta: &str,
    proxy_url: Option<&str>,
    device: &DeviceIdentity,
) -> Result<DashboardLinks> {
    // Same device and same account details as every later payload — this request
    // used to invent its own rid and claim a different country and age.
    let rid = device.rid.clone();
    let hash = hash_string(&format!("{}RT", device.mac));
    let klv = compute_klv(
        &login_info.game_version,
        &login_info.protocol.to_string(),
        &rid,
        hash,
    );

    let body = build_dashboard_body(login_info, meta, device, &klv, hash);

    let agent = if let Some(p) = proxy_url {
        let proxy = ureq::Proxy::new(p)?;
        ureq::Agent::new_with_config(ureq::config::Config::builder().proxy(Some(proxy)).timeout_global(Some(Duration::from_secs(20))).build())
    } else {
        ureq::Agent::new_with_config(ureq::config::Config::builder().timeout_global(Some(Duration::from_secs(20))).build())
    };

    let html = agent
        .post(format!(
            "https://{}/player/login/dashboard?valKey=40db4045f2d8c572efe8c4a060605726",
            login_url
        ))
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)",
        )
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(body)?
        .body_mut()
        .read_to_string()?;

    if html.trim_start().starts_with('{') {
        let msg = serde_json::from_str::<serde_json::Value>(&html)
            .ok()
            .and_then(|v| v["message"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| html.clone());
        return Err(format!("Dashboard returned error: {msg}").into());
    }

    let document = Html::parse_document(&html);
    let link_selector = Selector::parse("a")
        .map_err(|e| format!("Failed to parse selector: {e}"))?;

    let mut apple_href = None;
    let mut google_href = None;
    let mut growtopia_href = None;

    for element in document.select(&link_selector) {
        if let Some(onclick) = element.value().attr("onclick") {
            if onclick.contains("optionChose('Apple')") {
                apple_href = element.value().attr("href").map(Into::into);
            } else if onclick.contains("optionChose('Google')") {
                google_href = element.value().attr("href").map(Into::into);
            } else if onclick.contains("optionChose('Grow')") {
                growtopia_href = element.value().attr("href").map(Into::into);
            }
        }
    }

    Ok(DashboardLinks {
        apple: apple_href,
        google: google_href,
        growtopia: growtopia_href,
    })
}

/// The form body of the dashboard request. Split out so a test can check it against
/// the same persona the later login payloads use.
fn build_dashboard_body(
    login_info: &LoginInfo,
    meta: &str,
    device: &DeviceIdentity,
    klv: &str,
    hash: i32,
) -> String {
    build_pipe_body(&[
        ("tankIDName",    ""),
        ("tankIDPass",    ""),
        ("requestedName", ""),
        ("f",             "1"),
        ("protocol",      &login_info.protocol.to_string()),
        ("game_version",  &login_info.game_version),
        ("cbits",         &device.cbits.to_string()),
        ("player_age",    &device.player_age.to_string()),
        ("GDPR",          &device.gdpr.to_string()),
        ("FCMToken",      ""),
        ("category",      "_-5100"),
        ("totalPlaytime", "0"),
        ("klv",           klv),
        ("meta",          meta),
        ("fhash",         &fhash().to_string()),
        ("rid",           &device.rid),
        ("platformID",    "2"),
        ("deviceVersion", "0"),
        ("country",       &device.country),
        ("hash",          &hash.to_string()),
        ("mac",           &device.mac),
        ("wk",            &device.wk),
    ])
}

fn build_pipe_body(fields: &[(&str, &str)]) -> String {
    fields.iter().map(|(k, v)| format!("{k}|{v}\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_body_matches_the_stored_persona() {
        let mut device = DeviceIdentity::generate();
        device.country = "id".into();
        device.player_age = 27;
        let login_info = LoginInfo {
            protocol: 225,
            game_version: "5.51".into(),
        };

        let body = build_dashboard_body(&login_info, "META", &device, "KLV", 7);
        let field = |key: &str| -> String {
            body.lines()
                .find_map(|l| l.strip_prefix(&format!("{key}|")))
                .unwrap_or_else(|| panic!("{key} missing"))
                .to_string()
        };

        assert_eq!(field("country"), "id");
        assert_eq!(field("player_age"), "27");
        assert_eq!(field("GDPR"), device.gdpr.to_string());
        assert_eq!(field("cbits"), device.cbits.to_string());
        assert_eq!(field("rid"), device.rid);
        assert_eq!(field("mac"), device.mac);
        assert_eq!(field("wk"), device.wk);
    }
}
