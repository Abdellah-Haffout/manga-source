use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Manga {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_titles: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub views: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_chapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genres: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub is_nsfw: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manga_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_year: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Chapter {
    pub id: String,
    pub chapter_number: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scanlator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub views: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub index: usize,
    pub filename: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputFormat {
    Folder,
    Cbz,
    Pdf,
}

#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub output_dir: PathBuf,
    pub format: OutputFormat,
    pub concurrent_downloads: usize,
    #[allow(dead_code)]
    pub language: Option<String>,
    pub cookies: Option<String>,
    pub user_agent: Option<String>,
    pub use_aria2: bool,
    pub compress_webp: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GenreOption {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SortOption {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MangaFilter {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub genres: Option<Vec<String>>,
    #[serde(default)]
    pub status: Option<String>, // "all", "ongoing", "completed", "hiatus", "cancelled"
    #[serde(default)]
    pub order_by: Option<String>, // "latest", "rating", "views", "alphabet", "newest"
    #[serde(default)]
    pub manga_type: Option<String>, // "all", "manga", "manhwa", "manhua"
    #[serde(default)]
    pub demographic: Option<String>, // "shounen", "shoujo", "seinen", "josei"
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub is_nsfw: Option<bool>,
    #[serde(default)]
    pub page: Option<u32>,
    #[serde(default)]
    pub limit: Option<u32>,
}

