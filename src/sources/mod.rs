pub mod json_source;
pub mod mangadex;

use anyhow::Result;
use async_trait::async_trait;
use crate::models::{Chapter, Manga, Page};

#[async_trait]
pub trait MangaSource: Send + Sync {
    fn name(&self) -> &str;
    async fn search(&self, query: &str) -> Result<Vec<Manga>>;
    async fn get_latest(&self) -> Result<Vec<Manga>> {
        self.search("").await
    }
    async fn get_chapters(&self, manga_id: &str, lang: Option<&str>) -> Result<Vec<Chapter>>;
    async fn get_pages(&self, chapter_id: &str) -> Result<Vec<Page>>;
}
