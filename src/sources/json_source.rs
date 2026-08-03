use crate::cookies::CookieStore;
use crate::models::{Chapter, Manga, Page};
use crate::sources::MangaSource;
use anyhow::Result;
use async_trait::async_trait;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegexExtractor {
    pub pattern: String,
    pub id_group: usize,
    pub title_group: Option<usize>,
    pub cover_group: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestStep {
    pub url_template: String,
    pub method: Option<String>,
    pub regex: RegexExtractor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonSourceConfig {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub user_agent: Option<String>,
    pub search: RequestStep,
    pub latest: Option<RequestStep>,
    pub chapters: RequestStep,
    pub pages: RequestStep,
}

pub struct JsonSource {
    config: JsonSourceConfig,
    client: Client,
}

impl JsonSource {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: JsonSourceConfig = serde_json::from_str(&content)?;
        Self::from_config(config)
    }

    pub fn from_config(config: JsonSourceConfig) -> Result<Self> {
        let store = CookieStore::load();
        let session = store.get_session_for_domain(&config.base_url);

        let ua = session
            .and_then(|s| s.user_agent.as_deref())
            .unwrap_or_else(|| {
                config
                    .user_agent
                    .as_deref()
                    .unwrap_or("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
            });

        let mut headers = reqwest::header::HeaderMap::new();
        store.apply_headers_for_url(&config.base_url, &mut headers);

        let client = Client::builder()
            .user_agent(ua)
            .default_headers(headers)
            .danger_accept_invalid_certs(true)
            .build()?;

        Ok(Self { config, client })
    }

    pub fn id(&self) -> &str {
        &self.config.id
    }
}

#[async_trait]
impl MangaSource for JsonSource {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn search(&self, query: &str) -> Result<Vec<Manga>> {
        let url = self
            .config
            .search
            .url_template
            .replace("{base_url}", &self.config.base_url)
            .replace("{query}", &query.replace(' ', "+"));

        let req = match self.config.search.method.as_deref().unwrap_or("GET").to_uppercase().as_str() {
            "POST" => self.client.post(&url),
            _ => self.client.get(&url),
        };

        let html = req.send().await?.text().await?;
        let re = Regex::new(&self.config.search.regex.pattern)?;

        let mut results = Vec::new();
        let mut seen = HashSet::new();

        for cap in re.captures_iter(&html) {
            let id = cap
                .get(self.config.search.regex.id_group)
                .map(|m| m.as_str())
                .unwrap_or("");

            let title = if let Some(grp) = self.config.search.regex.title_group {
                cap.get(grp).map(|m| m.as_str().trim().to_string()).unwrap_or_else(|| id.to_string())
            } else {
                id.to_string()
            };

            let cover_url = if let Some(grp) = self.config.search.regex.cover_group {
                cap.get(grp).map(|m| m.as_str().trim().to_string())
            } else {
                None
            };

            if !id.is_empty() && seen.insert(id.to_string()) {
                results.push(Manga {
                    id: id.to_string(),
                    title,
                    description: None,
                    cover_url,
                    author: None,
                });
            }
        }

        Ok(results)
    }

    async fn get_latest(&self) -> Result<Vec<Manga>> {
        let step = self.config.latest.as_ref().unwrap_or(&self.config.search);
        let url = step
            .url_template
            .replace("{base_url}", &self.config.base_url)
            .replace("{query}", "");

        let req = match step.method.as_deref().unwrap_or("GET").to_uppercase().as_str() {
            "POST" => self.client.post(&url),
            _ => self.client.get(&url),
        };

        let html = req.send().await?.text().await?;
        let re = Regex::new(&step.regex.pattern)?;

        let mut results = Vec::new();
        let mut seen = HashSet::new();

        for cap in re.captures_iter(&html) {
            let id = cap
                .get(step.regex.id_group)
                .map(|m| m.as_str())
                .unwrap_or("");

            let title = if let Some(grp) = step.regex.title_group {
                cap.get(grp).map(|m| m.as_str().trim().to_string()).unwrap_or_else(|| id.to_string())
            } else {
                id.to_string()
            };

            let cover_url = if let Some(grp) = step.regex.cover_group {
                cap.get(grp).map(|m| m.as_str().trim().to_string())
            } else {
                None
            };

            if !id.is_empty() && seen.insert(id.to_string()) {
                results.push(Manga {
                    id: id.to_string(),
                    title,
                    description: None,
                    cover_url,
                    author: None,
                });
            }
        }

        Ok(results)
    }

    async fn get_chapters(&self, manga_id: &str, _lang: Option<&str>) -> Result<Vec<Chapter>> {
        let url = self
            .config
            .chapters
            .url_template
            .replace("{base_url}", &self.config.base_url)
            .replace("{manga_id}", manga_id);

        let req = match self.config.chapters.method.as_deref().unwrap_or("GET").to_uppercase().as_str() {
            "POST" => self.client.post(&url),
            _ => self.client.get(&url),
        };

        let html = req.send().await?.text().await?;
        let pattern = self
            .config
            .chapters
            .regex
            .pattern
            .replace("{manga_id}", &regex::escape(manga_id));

        let re = Regex::new(&pattern)?;
        let mut chapters = Vec::new();
        let mut seen = HashSet::new();

        for cap in re.captures_iter(&html) {
            let ch_id = cap
                .get(self.config.chapters.regex.id_group)
                .map(|m| m.as_str().trim())
                .unwrap_or("");

            let title_str = if let Some(grp) = self.config.chapters.regex.title_group {
                cap.get(grp).map(|m| m.as_str().trim().to_string())
            } else {
                None
            };

            let clean_num = ch_id
                .split('-')
                .last()
                .unwrap_or(ch_id)
                .trim()
                .to_string();

            if !ch_id.is_empty() && seen.insert(ch_id.to_string()) {
                let full_id = if ch_id.starts_with(manga_id) {
                    ch_id.to_string()
                } else {
                    format!("{}/{}", manga_id, ch_id)
                };

                chapters.push(Chapter {
                    id: full_id,
                    chapter_number: clean_num,
                    title: title_str,
                    language: None,
                    scanlator: None,
                });
            }
        }

        chapters.reverse();
        Ok(chapters)
    }

    async fn get_pages(&self, chapter_id: &str) -> Result<Vec<Page>> {
        let url = self
            .config
            .pages
            .url_template
            .replace("{base_url}", &self.config.base_url)
            .replace("{chapter_id}", chapter_id);

        let req = match self.config.pages.method.as_deref().unwrap_or("GET").to_uppercase().as_str() {
            "POST" => self.client.post(&url),
            _ => self.client.get(&url),
        };

        let html = req.send().await?.text().await?;
        let re = Regex::new(&self.config.pages.regex.pattern)?;

        let mut pages = Vec::new();

        for (idx, cap) in re.captures_iter(&html).enumerate() {
            if let Some(src_match) = cap.get(self.config.pages.regex.id_group) {
                let img_url = src_match.as_str().trim().to_string();
                let ext = img_url.split('.').last().unwrap_or("jpg").split('?').next().unwrap_or("jpg").to_string();
                pages.push(Page {
                    index: idx + 1,
                    filename: format!("{:03}.{}", idx + 1, ext),
                    url: img_url,
                });
            }
        }

        Ok(pages)
    }
}
