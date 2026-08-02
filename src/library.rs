use crate::downloader::Downloader;
use crate::models::{DownloadOptions, OutputFormat};
use crate::sources::MangaSource;
use anyhow::Result;
use indicatif::MultiProgress;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryItem {
    pub manga_id: String,
    pub source_id: String,
    pub title: String,
    pub cover_url: Option<String>,
    pub last_downloaded_chapter: Option<String>,
    pub preferred_format: String,
    pub output_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LibraryStore {
    pub items: Vec<LibraryItem>,
}

impl LibraryStore {
    pub fn default_path() -> PathBuf {
        PathBuf::from("./library.json")
    }

    pub fn load() -> Self {
        let path = Self::default_path();
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(store) = serde_json::from_str::<LibraryStore>(&content) {
                    return store;
                }
            }
        }
        LibraryStore::default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::default_path();
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn add(&mut self, item: LibraryItem) -> Result<()> {
        if let Some(pos) = self.items.iter().position(|i| i.manga_id == item.manga_id && i.source_id == item.source_id) {
            self.items[pos] = item;
        } else {
            self.items.push(item);
        }
        self.save()
    }

    pub fn remove(&mut self, manga_id: &str, source_id: Option<&str>) -> Result<bool> {
        let original_len = self.items.len();
        self.items.retain(|i| {
            if let Some(src) = source_id {
                !(i.manga_id == manga_id && i.source_id == src)
            } else {
                i.manga_id != manga_id
            }
        });
        let removed = self.items.len() < original_len;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    pub async fn check_and_update_all<F>(&mut self, get_source_fn: F) -> Result<Vec<String>>
    where
        F: Fn(&str) -> Box<dyn MangaSource>,
    {
        let mut updates_summary = Vec::new();
        let mp = MultiProgress::new();

        for item in self.items.iter_mut() {
            let source = get_source_fn(&item.source_id);
            if let Ok(chapters) = source.get_chapters(&item.manga_id, None).await {
                if chapters.is_empty() {
                    continue;
                }

                let last_num: f32 = item
                    .last_downloaded_chapter
                    .as_deref()
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0.0);

                let new_chapters: Vec<_> = chapters
                    .into_iter()
                    .filter(|c| {
                        let num: f32 = c.chapter_number.parse().unwrap_or(-1.0);
                        num > last_num
                    })
                    .collect();

                if !new_chapters.is_empty() {
                    let fmt = match item.preferred_format.to_lowercase().as_str() {
                        "cbz" => OutputFormat::Cbz,
                        "pdf" => OutputFormat::Pdf,
                        _ => OutputFormat::Folder,
                    };

                    let options = DownloadOptions {
                        output_dir: PathBuf::from(&item.output_dir),
                        format: fmt,
                        concurrent_downloads: 4,
                        language: None,
                        cookies: None,
                        user_agent: None,
                        use_aria2: false,
                        compress_webp: false,
                    };

                    let downloader = Downloader::new(options);
                    let mut downloaded_count = 0;
                    let mut newest_ch_num = item.last_downloaded_chapter.clone();

                    for ch in &new_chapters {
                        if let Ok(_) = downloader.download_chapter(source.as_ref(), &item.title, ch, &mp).await {
                            downloaded_count += 1;
                            newest_ch_num = Some(ch.chapter_number.clone());
                        }
                    }

                    if downloaded_count > 0 {
                        item.last_downloaded_chapter = newest_ch_num;
                        let msg = format!(
                            "Updated '{}' ({}): downloaded {} new chapters (latest: Ch. {})",
                            item.title,
                            item.source_id,
                            downloaded_count,
                            item.last_downloaded_chapter.as_deref().unwrap_or("?")
                        );
                        updates_summary.push(msg);
                    }
                }
            }
        }

        self.save()?;
        Ok(updates_summary)
    }
}
