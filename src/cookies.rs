use anyhow::Result;
use reqwest::header::{HeaderMap, HeaderValue, COOKIE, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieSession {
    pub domain: String,
    pub cookie_string: String,
    pub user_agent: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CookieStore {
    pub sessions: Vec<CookieSession>,
}

impl CookieStore {
    pub fn default_path() -> PathBuf {
        PathBuf::from("./cookies.json")
    }

    pub fn load() -> Self {
        let path = Self::default_path();
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(store) = serde_json::from_str::<CookieStore>(&content) {
                    return store;
                }
            }
        }
        CookieStore::default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::default_path();
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn get_session_for_domain(&self, domain_or_url: &str) -> Option<&CookieSession> {
        let clean = domain_or_url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or(domain_or_url);

        self.sessions
            .iter()
            .find(|s| clean.contains(&s.domain) || s.domain.contains(clean))
    }

    pub fn set_session(&mut self, session: CookieSession) -> Result<()> {
        if let Some(pos) = self.sessions.iter().position(|s| s.domain == session.domain) {
            self.sessions[pos] = session;
        } else {
            self.sessions.push(session);
        }
        self.save()
    }

    pub fn clear_domain(&mut self, domain: &str) -> Result<bool> {
        let original_len = self.sessions.len();
        self.sessions.retain(|s| s.domain != domain);
        let removed = self.sessions.len() < original_len;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    pub fn apply_headers_for_url(&self, url: &str, headers: &mut HeaderMap) {
        if let Some(session) = self.get_session_for_domain(url) {
            if let Ok(val) = HeaderValue::from_str(&session.cookie_string) {
                headers.insert(COOKIE, val);
            }
            if let Some(ua) = &session.user_agent {
                if let Ok(val) = HeaderValue::from_str(ua) {
                    headers.insert(USER_AGENT, val);
                }
            }
        }
    }
}
