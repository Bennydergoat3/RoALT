//! How many people use RoALT now, shown beside the Discord button while `settings.online_count`
//! is on. Each ping sends only a random id made once per launch to RoALT's counter (a Cloudflare
//! Worker, `server/online-counter`), which answers how many ids it saw in the last 5 minutes.

use std::sync::LazyLock;
use std::time::Duration;

use reqwest::redirect::Policy;
use serde::Deserialize;
use uuid::Uuid;

const ONLINE_URL: &str = "https://roalt-online.roalt-counterlive.workers.dev/ping";

/// One per launch: the counter tells runs apart, never people.
static LAUNCH_ID: LazyLock<Uuid> = LazyLock::new(Uuid::new_v4);

#[derive(Deserialize)]
struct Answer {
    online: u32,
}

/// The counter's answer, or None (logged) when it can't be reached or read.
pub async fn ping() -> Option<u32> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("RoALT/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .inspect_err(|e| log::warn!("online: http client unavailable: {e}"))
        .ok()?;
    let resp = http
        .post(ONLINE_URL)
        .json(&serde_json::json!({ "id": *LAUNCH_ID }))
        .send()
        .await
        .map_err(|e| log::info!("online: counter unreachable: {}", e.without_url()))
        .ok()?;
    if !resp.status().is_success() {
        log::info!("online: counter answered HTTP {}", resp.status().as_u16());
        return None;
    }
    let body = resp.text().await.map_err(|e| log::info!("online: counter cut off: {}", e.without_url())).ok()?;
    parse(&body)
}

fn parse(body: &str) -> Option<u32> {
    serde_json::from_str::<Answer>(body).inspect_err(|e| log::info!("online: counter's answer unreadable: {e}")).ok().map(|a| a.online)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer() {
        assert_eq!(parse(r#"{"online":128}"#), Some(128));
        assert_eq!(parse(r#"{"online":-1}"#), None);
        assert_eq!(parse("Not found"), None);
        assert_eq!(serde_json::json!({ "id": *LAUNCH_ID })["id"].as_str().map(str::len), Some(36), "a plain hyphenated uuid");
    }

    /// Against the deployed counter: `cargo test online::tests::live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live() {
        let n = ping().await;
        println!("online: {n:?}");
        assert!(n.is_some_and(|n| n >= 1));
    }
}
