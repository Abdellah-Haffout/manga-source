pub mod json_source;
pub mod mangadex;

use anyhow::Result;
use async_trait::async_trait;
use crate::models::{Chapter, GenreOption, Manga, MangaFilter, Page, SortOption};

#[async_trait]
pub trait MangaSource: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn base_url(&self) -> &str {
        ""
    }
    fn languages(&self) -> Vec<String> {
        vec!["en".to_string()]
    }
    fn is_nsfw(&self) -> bool {
        false
    }
    fn tags(&self) -> Vec<String> {
        vec![]
    }
    fn icon_url(&self) -> Option<String> {
        None
    }

    fn available_genres(&self) -> Vec<GenreOption> {
        vec![]
    }

    fn available_sort_orders(&self) -> Vec<SortOption> {
        vec![
            SortOption { id: "latest".to_string(), name: "🔥 أحدث التحديثات (Latest Updates)".to_string() },
            SortOption { id: "rating".to_string(), name: "⭐ الأعلى تقييماً (Highest Rating)".to_string() },
            SortOption { id: "views".to_string(), name: "👁️ الأكثر مشاهدة وشعبية (Most Views)".to_string() },
            SortOption { id: "alphabet".to_string(), name: "🔤 أبجدي (A-Z / أ-ي)".to_string() },
            SortOption { id: "newest".to_string(), name: "🆕 الأحدث إضافة (Recently Added)".to_string() },
        ]
    }

    async fn search(&self, query: &str) -> Result<Vec<Manga>>;
    async fn get_latest(&self) -> Result<Vec<Manga>> {
        self.search("").await
    }
    async fn filter_manga(&self, filter: &MangaFilter) -> Result<Vec<Manga>> {
        if let Some(q) = &filter.query {
            if !q.trim().is_empty() {
                return self.search(q).await;
            }
        }
        self.get_latest().await
    }
    async fn get_manga_details(&self, _manga_id: &str) -> Result<Option<Manga>> {
        Ok(None)
    }
    async fn get_chapters(&self, manga_id: &str, lang: Option<&str>) -> Result<Vec<Chapter>>;
    async fn get_pages(&self, chapter_id: &str) -> Result<Vec<Page>>;
}
