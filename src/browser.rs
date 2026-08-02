use anyhow::Result;
use reqwest::header::{HeaderMap, HeaderValue, COOKIE, USER_AGENT};
use std::process::Command;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BrowserSession {
    pub cookies: Option<String>,
    pub user_agent: Option<String>,
}

impl BrowserSession {
    #[allow(dead_code)]
    pub fn new(cookies: Option<String>, user_agent: Option<String>) -> Self {
        Self { cookies, user_agent }
    }

    #[allow(dead_code)]
    pub fn build_headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();

        if let Some(cookie_str) = &self.cookies {
            headers.insert(COOKIE, HeaderValue::from_str(cookie_str)?);
        }

        if let Some(ua_str) = &self.user_agent {
            headers.insert(USER_AGENT, HeaderValue::from_str(ua_str)?);
        }

        Ok(headers)
    }

    pub fn launch_interactive_bypass(url: &str) -> Result<()> {
        println!("Launching default web browser to pass Cloudflare / CAPTCHA at {} ...", url);
        open::that(url)?;
        println!("Please complete the Cloudflare challenge in your browser window.");
        Ok(())
    }

    #[allow(dead_code)]
    pub fn check_system_browsers() -> Vec<String> {
        let mut found = Vec::new();

        let candidates = [
            ("google-chrome", "Google Chrome"),
            ("chromium", "Chromium"),
            ("firefox", "Mozilla Firefox"),
            ("brave-browser", "Brave"),
            ("microsoft-edge", "Microsoft Edge"),
        ];

        for (cmd, name) in candidates {
            if Command::new("which").arg(cmd).output().map_or(false, |o| o.status.success()) {
                found.push(name.to_string());
            }
        }

        found
    }
}
