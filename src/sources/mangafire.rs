use crate::models::{Chapter, Manga, Page};
use crate::sources::MangaSource;
use anyhow::Result;
use async_trait::async_trait;
use regex::Regex;
use reqwest::Client;
use std::collections::HashSet;

pub struct MangaFireSource {
    client: Client,
    base_url: String,
}

impl MangaFireSource {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
            .build()
            .unwrap_or_default();
        Self {
            client,
            base_url: "https://mangafire.to".to_string(),
        }
    }
}

impl Default for MangaFireSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MangaSource for MangaFireSource {
    fn name(&self) -> &str {
        "MangaFire (mangafire.to)"
    }

    async fn search(&self, query: &str) -> Result<Vec<Manga>> {
        let url = format!("{}/filter?keyword={}", self.base_url, query.replace(' ', "+"));
        let html = self.client.get(&url).send().await?.text().await?;

        let re_item = Regex::new(r#"href="/manga/([^"]+)"[^>]*class="[^"]*title[^"]*"[^>]*>([^<]+)</a>"#)?;
        let mut results = Vec::new();
        let mut seen = HashSet::new();

        for cap in re_item.captures_iter(&html) {
            let slug = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            let title = cap.get(2).map(|m| m.as_str().trim()).unwrap_or("Unknown");

            if !slug.is_empty() && seen.insert(slug.to_string()) {
                results.push(Manga {
                    id: slug.to_string(),
                    title: title.to_string(),
                    description: None,
                    cover_url: None,
                    author: None,
                });
            }
        }

        Ok(results)
    }

    async fn get_chapters(&self, manga_id: &str, _lang: Option<&str>) -> Result<Vec<Chapter>> {
        let url = format!("{}/manga/{}", self.base_url, manga_id);
        let html = self.client.get(&url).send().await?.text().await?;

        let re_ch = Regex::new(r#"href="/read/([^/]+/en/chapter-([0-9.]+))"[^>]*>([\s\S]*?)</a>"#)?;
        let mut chapters = Vec::new();
        let mut seen = HashSet::new();

        for cap in re_ch.captures_iter(&html) {
            let read_id = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            let ch_num = cap.get(2).map(|m| m.as_str().trim()).unwrap_or("0");

            if !read_id.is_empty() && seen.insert(read_id.to_string()) {
                chapters.push(Chapter {
                    id: read_id.to_string(),
                    chapter_number: ch_num.to_string(),
                    title: None,
                    language: Some("en".to_string()),
                    scanlator: None,
                });
            }
        }

        chapters.reverse();
        Ok(chapters)
    }

    async fn get_pages(&self, chapter_id: &str) -> Result<Vec<Page>> {
        let url = format!("{}/read/{}", self.base_url, chapter_id);
        let html = self.client.get(&url).send().await?.text().await?;

        let re_img = Regex::new(r#"data-url="([^"]+)""#)?;
        let mut pages = Vec::new();

        for (idx, cap) in re_img.captures_iter(&html).enumerate() {
            if let Some(src_match) = cap.get(1) {
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
