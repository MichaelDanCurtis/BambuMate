//! Text for HMS and `print_error` codes.
//!
//! The text comes from Bambu's public error list: a static JSON document
//! over plain HTTPS that needs no account (`https://e.bambulab.com/query.php`,
//! the source ha-bambulab's `scripts/update_error_text.py` uses). It is
//! fetched on first need and cached in app data for 7 days. With no network
//! and no cache, an error shows its code and a link to Bambu's wiki.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::state::PrinterState;

pub const HMS_URL: &str = "https://e.bambulab.com/query.php";
const CACHE_TTL_SECS: i64 = 7 * 24 * 60 * 60;
/// After a failed fetch, wait this long before trying again.
const RETRY_AFTER: Duration = Duration::from_secs(600);
const LANG: &str = "en";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct HmsCache {
    fetched_at: i64,
    device_hms: HashMap<String, String>,
    device_error: HashMap<String, String>,
}

/// One active error, ready to show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorView {
    /// `hms` or `print_error`.
    pub kind: String,
    /// `0300_0100_0001_0007` (HMS) or `0300_400C` (print error).
    pub code: String,
    /// `None` when the text is unknown or not loaded.
    pub text: Option<String>,
    pub wiki_url: String,
}

pub struct HmsCatalog {
    base_url: String,
    cache_path: PathBuf,
    /// The first three characters of the serial: Bambu's list for that model
    /// has codes the generic list lacks. `None` fetches the generic list.
    model_prefix: Option<String>,
    cache: Mutex<Option<HmsCache>>,
    last_failed_fetch: Mutex<Option<Instant>>,
    http: reqwest::Client,
}

impl HmsCatalog {
    /// `cache_dir` is the app data dir; the file is `hms_en[_<prefix>].json`.
    pub fn new(cache_dir: PathBuf, serial: Option<&str>) -> Self {
        Self::with_base_url(cache_dir, serial, HMS_URL)
    }

    pub fn with_base_url(cache_dir: PathBuf, serial: Option<&str>, base_url: &str) -> Self {
        let model_prefix = serial
            .map(|s| s.chars().take(3).collect::<String>())
            .filter(|p| p.len() == 3 && p.chars().all(|c| c.is_ascii_alphanumeric()));
        let file = match &model_prefix {
            Some(p) => format!("hms_{LANG}_{p}.json"),
            None => format!("hms_{LANG}.json"),
        };
        Self {
            base_url: base_url.to_string(),
            cache_path: cache_dir.join(file),
            model_prefix,
            cache: Mutex::new(None),
            last_failed_fetch: Mutex::new(None),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
        }
    }

    /// True when error text is available in memory.
    pub fn is_loaded(&self) -> bool {
        self.cache.lock().unwrap().is_some()
    }

    /// Makes the text available: memory, then a fresh disk cache, then the
    /// network, then a stale disk cache. Returns whether text is available.
    pub async fn ensure_loaded(&self, now_secs: i64) -> bool {
        if let Some(c) = self.cache.lock().unwrap().as_ref() {
            if now_secs - c.fetched_at < CACHE_TTL_SECS {
                return true;
            }
        }
        let on_disk = std::fs::read(&self.cache_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<HmsCache>(&b).ok());
        if let Some(c) = &on_disk {
            if now_secs - c.fetched_at < CACHE_TTL_SECS {
                *self.cache.lock().unwrap() = on_disk;
                return true;
            }
        }
        let recently_failed = self
            .last_failed_fetch
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed() < RETRY_AFTER);
        if !recently_failed {
            match self.fetch(now_secs).await {
                Ok(fresh) => {
                    if let Some(dir) = self.cache_path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    if let Ok(bytes) = serde_json::to_vec(&fresh) {
                        if let Err(e) = std::fs::write(&self.cache_path, bytes) {
                            tracing::debug!("could not write HMS cache: {e}");
                        }
                    }
                    *self.cache.lock().unwrap() = Some(fresh);
                    return true;
                }
                Err(e) => {
                    tracing::debug!("HMS list fetch failed: {e}");
                    *self.last_failed_fetch.lock().unwrap() = Some(Instant::now());
                }
            }
        }
        if on_disk.is_some() {
            *self.cache.lock().unwrap() = on_disk;
            return true;
        }
        self.is_loaded()
    }

    async fn fetch(&self, now_secs: i64) -> Result<HmsCache, String> {
        let mut query = vec![("lang", LANG.to_string())];
        if let Some(p) = &self.model_prefix {
            query.push(("d", p.clone()));
        }
        let body: Value = self
            .http
            .get(&self.base_url)
            .query(&query)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        if body.get("result").and_then(Value::as_i64) != Some(0) {
            return Err("HMS list request was refused".into());
        }
        let table = |kind: &str| -> HashMap<String, String> {
            body.pointer(&format!("/data/{kind}/{LANG}"))
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|e| {
                            let code = e.get("ecode")?.as_str()?.to_ascii_uppercase();
                            let text = e.get("intro")?.as_str()?.trim().to_string();
                            (!text.is_empty()).then_some((code, text))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let cache = HmsCache {
            fetched_at: now_secs,
            device_hms: table("device_hms"),
            device_error: table("device_error"),
        };
        if cache.device_hms.is_empty() && cache.device_error.is_empty() {
            return Err("HMS list was empty".into());
        }
        Ok(cache)
    }

    /// Every active error in `state`, with text when it's known.
    pub fn describe(&self, state: &PrinterState, model: Option<&str>) -> Vec<ErrorView> {
        let cache = self.cache.lock().unwrap();
        let mut out: Vec<ErrorView> = state
            .hms
            .iter()
            .map(|h| {
                let code = h.display();
                ErrorView {
                    kind: "hms".into(),
                    text: cache
                        .as_ref()
                        .and_then(|c| c.device_hms.get(&code.replace('_', "")).cloned()),
                    wiki_url: hms_wiki_url(&code, model),
                    code,
                }
            })
            .collect();
        if let Some(err) = state.print_error {
            let hex = format!("{err:08X}");
            out.push(ErrorView {
                kind: "print_error".into(),
                code: format!("{}_{}", &hex[..4], &hex[4..]),
                text: cache
                    .as_ref()
                    .and_then(|c| c.device_error.get(&hex).cloned()),
                wiki_url: WIKI_HMS_HOME.into(),
            });
        }
        out
    }
}

const WIKI_HMS_HOME: &str = "https://wiki.bambulab.com/en/hms/home";

/// The wiki page for one HMS code. The path segment is the model family,
/// following the links ha-bambulab collects in `hms_error_text/wiki_links.json`.
pub fn hms_wiki_url(code: &str, model: Option<&str>) -> String {
    let m = model.unwrap_or("").to_ascii_uppercase().replace(' ', "");
    let family = match m.as_str() {
        "H2D" | "H2DPRO" => "h2",
        "H2C" => "h2c",
        "H2S" => "h2s",
        "X2D" => "x2d",
        "P2S" => "p2s",
        "A1" | "A1MINI" => "a1",
        "P1P" | "P1S" => "p1",
        _ => "x1",
    };
    format!("https://wiki.bambulab.com/en/{family}/troubleshooting/hmscode/{code}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::state::HmsCode;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const NOW: i64 = 1_790_000_000;
    const BODY: &str = r#"{"result":0,"t":1790657978,"ver":202609231145,"data":{
        "device_hms":{"ver":202609231145,"en":[
            {"ecode":"0300010000010007","intro":"The heatbed temperature is abnormal; the sensor may have an open circuit."},
            {"ecode":"0500060000020045","intro":""}]},
        "device_error":{"ver":202609231145,"en":[
            {"ecode":"0300400C","intro":"The task was canceled."}]}}}"#;

    /// A one-route HTTP server that counts requests and records the query.
    async fn stub() -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/query.php", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let (h, p) = (hits.clone(), paths.clone());
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                h.fetch_add(1, Ordering::SeqCst);
                let mut req = vec![0u8; 4096];
                let n = sock.read(&mut req).await.unwrap_or(0);
                let line = String::from_utf8_lossy(&req[..n])
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string();
                p.lock().unwrap().push(line);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    BODY.len(),
                    BODY
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        (url, hits, paths)
    }

    fn state_with_errors() -> PrinterState {
        PrinterState {
            hms: vec![HmsCode {
                attr: 0x0300_0100,
                code: 0x0001_0007,
            }],
            print_error: Some(0x0300_400C),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn fetches_on_first_need_and_writes_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let (url, hits, paths) = stub().await;
        let cat = HmsCatalog::with_base_url(dir.path().into(), Some("0948AB000000001"), &url);
        assert!(!cat.is_loaded());
        assert!(cat.ensure_loaded(NOW).await);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert!(paths.lock().unwrap()[0].contains("lang=en&d=094"));
        assert!(dir.path().join("hms_en_094.json").exists());

        let errors = cat.describe(&state_with_errors(), Some("H2D"));
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].code, "0300_0100_0001_0007");
        assert_eq!(
            errors[0].text.as_deref(),
            Some("The heatbed temperature is abnormal; the sensor may have an open circuit.")
        );
        assert_eq!(errors[1].kind, "print_error");
        assert_eq!(errors[1].code, "0300_400C");
        assert_eq!(errors[1].text.as_deref(), Some("The task was canceled."));

        assert!(cat.ensure_loaded(NOW + 60).await);
        assert_eq!(hits.load(Ordering::SeqCst), 1, "memory hit");
    }

    #[tokio::test]
    async fn a_fresh_disk_cache_is_used_without_fetching() {
        let dir = tempfile::tempdir().unwrap();
        let (url, hits, _) = stub().await;
        HmsCatalog::with_base_url(dir.path().into(), None, &url)
            .ensure_loaded(NOW)
            .await;
        let again = HmsCatalog::with_base_url(dir.path().into(), None, &url);
        assert!(again.ensure_loaded(NOW + 6 * 24 * 3600).await);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_cache_older_than_seven_days_is_fetched_again() {
        let dir = tempfile::tempdir().unwrap();
        let (url, hits, _) = stub().await;
        HmsCatalog::with_base_url(dir.path().into(), None, &url)
            .ensure_loaded(NOW)
            .await;
        let again = HmsCatalog::with_base_url(dir.path().into(), None, &url);
        assert!(again.ensure_loaded(NOW + 8 * 24 * 3600).await);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn offline_without_a_cache_falls_back_to_the_wiki_link() {
        let dir = tempfile::tempdir().unwrap();
        let spare = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/query.php", spare.local_addr().unwrap());
        drop(spare);
        let cat = HmsCatalog::with_base_url(dir.path().into(), None, &url);
        assert!(!cat.ensure_loaded(NOW).await);
        let errors = cat.describe(&state_with_errors(), Some("H2D"));
        assert_eq!(errors[0].text, None);
        assert_eq!(
            errors[0].wiki_url,
            "https://wiki.bambulab.com/en/h2/troubleshooting/hmscode/0300_0100_0001_0007"
        );
    }

    #[tokio::test]
    async fn offline_with_a_stale_cache_still_uses_it() {
        let dir = tempfile::tempdir().unwrap();
        let (url, _, _) = stub().await;
        HmsCatalog::with_base_url(dir.path().into(), None, &url)
            .ensure_loaded(NOW)
            .await;
        let spare = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = format!("http://{}/query.php", spare.local_addr().unwrap());
        drop(spare);
        let cat = HmsCatalog::with_base_url(dir.path().into(), None, &dead);
        assert!(cat.ensure_loaded(NOW + 30 * 24 * 3600).await);
        assert!(cat.describe(&state_with_errors(), None)[0].text.is_some());
    }

    #[test]
    fn wiki_links_follow_the_model_family() {
        let code = "0300_0100_0001_0007";
        assert!(hms_wiki_url(code, Some("H2D Pro")).contains("/en/h2/"));
        assert!(hms_wiki_url(code, Some("H2C")).contains("/en/h2c/"));
        assert!(hms_wiki_url(code, Some("X1 Carbon")).contains("/en/x1/"));
        assert!(hms_wiki_url(code, None).ends_with("/hmscode/0300_0100_0001_0007"));
    }
}
