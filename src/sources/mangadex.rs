use crate::models::{Chapter, Manga, Page};
use crate::sources::MangaSource;
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;

pub struct MangaDexSource {
    client: Client,
    base_url: String,
}

impl MangaDexSource {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("manga-source-rust/0.1.0")
            .build()
            .unwrap_or_default();
        Self {
            client,
            base_url: "https://api.mangadex.org".to_string(),
        }
    }
}

impl Default for MangaDexSource {
    fn default() -> Self {
        Self::new()
    }
}

// Serde DTOs for MangaDex API
#[derive(Deserialize)]
struct MangaListResponse {
    data: Vec<MangaData>,
}

#[derive(Deserialize)]
struct MangaData {
    id: String,
    attributes: MangaAttributes,
}

#[derive(Deserialize)]
struct MangaAttributes {
    title: HashMap<String, String>,
    description: Option<HashMap<String, String>>,
}

#[derive(Deserialize)]
struct ChapterFeedResponse {
    data: Vec<ChapterData>,
}

#[derive(Deserialize)]
struct ChapterData {
    id: String,
    attributes: ChapterAttributes,
}

#[derive(Deserialize)]
struct ChapterAttributes {
    chapter: Option<String>,
    title: Option<String>,
    #[serde(rename = "translatedLanguage")]
    translated_language: Option<String>,
}

#[derive(Deserialize)]
struct AtHomeResponse {
    #[serde(rename = "baseUrl")]
    base_url: String,
    chapter: AtHomeChapter,
}

#[derive(Deserialize)]
struct AtHomeChapter {
    hash: String,
    data: Vec<String>,
    #[serde(rename = "dataSaver", default)]
    data_saver: Vec<String>,
}

#[async_trait]
impl MangaSource for MangaDexSource {
    fn name(&self) -> &str {
        "MangaDex"
    }

    async fn search(&self, query: &str) -> Result<Vec<Manga>> {
        let url = format!("{}/manga", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&[("title", query), ("limit", "20")])
            .send()
            .await?
            .json::<MangaListResponse>()
            .await?;

        let mangas = resp
            .data
            .into_iter()
            .map(|item| {
                let title = item
                    .attributes
                    .title
                    .get("en")
                    .cloned()
                    .or_else(|| item.attributes.title.values().next().cloned())
                    .unwrap_or_else(|| "Unknown Title".to_string());

                let description = item
                    .attributes
                    .description
                    .and_then(|desc| desc.get("en").cloned().or_else(|| desc.values().next().cloned()));

                Manga {
                    id: item.id,
                    title,
                    description,
                    cover_url: None,
                    author: None,
                }
            })
            .collect();

        Ok(mangas)
    }

    async fn get_chapters(&self, manga_id: &str, lang: Option<&str>) -> Result<Vec<Chapter>> {
        let url = format!("{}/manga/{}/feed", self.base_url, manga_id);
        let target_lang = lang.unwrap_or("en");

        let resp = self
            .client
            .get(&url)
            .query(&[
                ("translatedLanguage[]", target_lang),
                ("order[chapter]", "asc"),
                ("limit", "500"),
            ])
            .send()
            .await?
            .json::<ChapterFeedResponse>()
            .await?;

        let chapters = resp
            .data
            .into_iter()
            .map(|item| {
                let ch_num = item.attributes.chapter.unwrap_or_else(|| "0".to_string());
                Chapter {
                    id: item.id,
                    chapter_number: ch_num,
                    title: item.attributes.title,
                    language: item.attributes.translated_language,
                    scanlator: None,
                }
            })
            .collect();

        Ok(chapters)
    }

    async fn get_pages(&self, chapter_id: &str) -> Result<Vec<Page>> {
        let url = format!("{}/at-home/server/{}", self.base_url, chapter_id);
        let resp = self
            .client
            .get(&url)
            .send()
            .await?
            .json::<AtHomeResponse>()
            .await?;

        let (files, quality) = if !resp.chapter.data.is_empty() {
            (resp.chapter.data, "data")
        } else {
            (resp.chapter.data_saver, "data-saver")
        };

        let base_image_url = format!("{}/{}/{}", resp.base_url, quality, resp.chapter.hash);
        let pages = files
            .into_iter()
            .enumerate()
            .map(|(idx, filename)| {
                let page_url = format!("{}/{}", base_image_url, filename);
                let ext = filename.split('.').last().unwrap_or("jpg");
                Page {
                    index: idx + 1,
                    url: page_url,
                    filename: format!("{:03}.{}", idx + 1, ext),
                }
            })
            .collect();

        Ok(pages)
    }
}
