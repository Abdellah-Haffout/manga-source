use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manga {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    pub author: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub id: String,
    pub chapter_number: String,
    pub title: Option<String>,
    pub language: Option<String>,
    pub scanlator: Option<String>,
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
