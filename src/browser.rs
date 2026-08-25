use anyhow::{anyhow, Result};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tokio::io::AsyncBufReadExt;
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::cookies::{CookieSession, CookieStore};

#[derive(Debug, Deserialize)]
struct BrowserVersionResponse {
    #[serde(rename = "User-Agent")]
    user_agent: Option<String>,
    #[serde(rename = "webSocketDebuggerUrl")]
    web_socket_debugger_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct PageTargetInfo {
    #[serde(rename = "type")]
    target_type: Option<String>,
    url: Option<String>,
    #[serde(rename = "webSocketDebuggerUrl")]
    web_socket_debugger_url: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct CdpCookie {
    name: String,
    value: String,
    domain: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CdpResult {
    cookies: Option<Vec<CdpCookie>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CdpResponse {
    id: Option<u64>,
    result: Option<CdpResult>,
    error: Option<serde_json::Value>,
}

pub struct BrowserSession;

impl BrowserSession {
    pub fn find_browser() -> Result<String> {
        let candidates = [
            "/usr/bin/brave",
            "brave",
            "brave-browser",
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
        ];

        for cmd in candidates {
            if Command::new("which").arg(cmd).output().map_or(false, |o| o.status.success()) {
                return Ok(cmd.to_string());
            }
            if std::path::Path::new(cmd).exists() {
                return Ok(cmd.to_string());
            }
        }

        Err(anyhow!("لم يتم العثور على متصفح مدعوم (Brave, Chrome, Chromium). يرجى تثبيت Brave أو Chrome."))
    }

    pub async fn launch_interactive_bypass(url_or_domain: &str) -> Result<()> {
        let clean_domain = url_or_domain
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or(url_or_domain)
            .to_string();

        let target_url = if url_or_domain.starts_with("http://") || url_or_domain.starts_with("https://") {
            url_or_domain.to_string()
        } else {
            format!("https://{}", clean_domain)
        };

        let browser_bin = Self::find_browser()?;
        let port = 9222;
        let profile_dir = PathBuf::from("/tmp/manga_browser_session");
        let _ = std::fs::create_dir_all(&profile_dir);

        println!("============================================================");
        println!("🚀 تشغيل المتصفح التلقائي لتجاوز حماية Cloudflare");
        println!("📍 الرابط المستهدف: {}", target_url);
        println!("🌐 المتصفح المستخدم: {}", browser_bin);
        println!("============================================================");
        println!("👉 تم فتح نافذة المتصفح. يرجى اجتياز التحقق في المتصفح.");
        println!("💡 نصيحة: يمكنك أيضاً الضغط على [Enter] في هذا التيرمينال فور تحميل الصفحة لحفظ الكوكيز فوراً!");
        println!("⏳ البرنامج يستمع بانتظار التقاط الكوكيز...");

        let mut child = Command::new(&browser_bin)
            .arg(format!("--remote-debugging-port={}", port))
            .arg(format!("--user-data-dir={}", profile_dir.display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg(&target_url)
            .spawn()
            .map_err(|e| anyhow!("فشل في تشغيل المتصفح: {}", e))?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()?;

        let mut ws_url = None;
        let mut user_agent = None;

        for _ in 0..20 {
            sleep(Duration::from_millis(500)).await;
            if let Ok(resp) = client.get(format!("http://127.0.0.1:{}/json/version", port)).send().await {
                if let Ok(info) = resp.json::<BrowserVersionResponse>().await {
                    user_agent = info.user_agent;
                    ws_url = info.web_socket_debugger_url;
                    break;
                }
            }
        }

        let ws_url = ws_url.ok_or_else(|| anyhow!("تعذر الاتصال بواجهة تصحيح المتصفح (CDP). تأكد من إغلاق أي نافذة سابقة للمتصفح."))?;
        let user_agent = user_agent.unwrap_or_else(|| "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36".to_string());

        let (mut ws_stream, _) = connect_async(&ws_url).await
            .map_err(|e| anyhow!("فشل الاتصال بـ WebSocket المتصفح: {}", e))?;

        let mut captured_cookies: Option<String> = None;
        let max_wait_secs = 120;
        let start_time = std::time::Instant::now();
        let mut req_id: u64 = 1;

        // Async stdin listener for manual Enter press
        let mut stdin_reader = tokio::io::BufReader::new(tokio::io::stdin()).lines();

        while start_time.elapsed().as_secs() < max_wait_secs {
            // Check if user pressed Enter in terminal
            let manual_trigger = tokio::select! {
                line = stdin_reader.next_line() => {
                    line.is_ok()
                },
                _ = sleep(Duration::from_millis(800)) => {
                    false
                }
            };

            // 1. Try Storage.getCookies on Browser target
            let msg_storage = json!({
                "id": req_id,
                "method": "Storage.getCookies"
            });
            req_id += 1;
            let _ = ws_stream.send(Message::Text(msg_storage.to_string().into())).await;

            // 2. Try Network.getAllCookies
            let msg_network = json!({
                "id": req_id,
                "method": "Network.getAllCookies"
            });
            req_id += 1;
            let _ = ws_stream.send(Message::Text(msg_network.to_string().into())).await;

            // Read CDP responses
            let mut domain_cookies: Vec<CdpCookie> = Vec::new();
            while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_millis(400), ws_stream.next()).await {
                if let Message::Text(text) = msg {
                    if let Ok(resp) = serde_json::from_str::<CdpResponse>(&text) {
                        if let Some(res) = resp.result {
                            if let Some(cookies) = res.cookies {
                                for c in cookies {
                                    let matches_domain = c.domain.as_deref().map_or(false, |d| {
                                        let clean_d = d.trim_start_matches('.');
                                        clean_domain.contains(clean_d) || clean_d.contains(&clean_domain)
                                    });
                                    if matches_domain || c.name == "cf_clearance" {
                                        if !domain_cookies.iter().any(|existing| existing.name == c.name) {
                                            domain_cookies.push(c);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let has_clearance = domain_cookies.iter().any(|c| c.name == "cf_clearance");
            let has_any_domain_cookies = !domain_cookies.is_empty();

            if has_clearance || (manual_trigger && has_any_domain_cookies) {
                let cookie_str = domain_cookies
                    .iter()
                    .map(|c| format!("{}={}", c.name, c.value))
                    .collect::<Vec<_>>()
                    .join("; ");

                captured_cookies = Some(cookie_str);
                break;
            } else if manual_trigger {
                println!("⚠️ تم الضغط على Enter لكن لم يتم العثور على كوكيز للنطاق {} بعد. تأكد من تحميل الصفحة واضغط Enter مجدداً.", clean_domain);
            }
        }

        let _ = child.kill();

        if let Some(cookie_string) = captured_cookies {
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let now = format!("Unix Timestamp {}", now_secs);
            let mut store = CookieStore::load();
            store.set_session(CookieSession {
                domain: clean_domain.clone(),
                cookie_string: cookie_string.clone(),
                user_agent: Some(user_agent.clone()),
                updated_at: now,
            })?;

            println!("============================================================");
            println!("🎉 تم التقاط الجلسة وتجاوز Cloudflare بنجاح!");
            println!("🌐 النطاق: {}", clean_domain);
            println!("🍪 الكوكيز: {}", if cookie_string.len() > 80 { format!("{}...", &cookie_string[..80]) } else { cookie_string });
            println!("📱 User-Agent: {}", user_agent);
            println!("💾 تم الحفظ التلقائي في cookies.json");
            println!("============================================================");
            Ok(())
        } else {
            Err(anyhow!("انتهت المهلة المحددة (120 ثانية) دون التقاط كوكي cf_clearance. يرجى إعادة المحاولة والضغط على Enter فور تحميل الموقع."))
        }
    }
}
