use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum QueueStatus {
    Pending,
    Downloading,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: String,
    pub source_id: String,
    pub manga_id: String,
    pub manga_title: String,
    pub chapter_id: String,
    pub chapter_number: String,
    pub format: String,
    pub output_dir: String,
    pub status: QueueStatus,
    pub downloaded_pages: usize,
    pub total_pages: usize,
    pub use_aria2: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueueStore {
    pub items: Vec<QueueItem>,
}

impl QueueStore {
    pub fn default_path() -> PathBuf {
        PathBuf::from("./queue.json")
    }

    pub fn load() -> Self {
        let path = Self::default_path();
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(store) = serde_json::from_str::<QueueStore>(&content) {
                    return store;
                }
            }
        }
        QueueStore::default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::default_path();
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn push(&mut self, item: QueueItem) -> Result<()> {
        if let Some(pos) = self.items.iter().position(|i| i.id == item.id) {
            self.items[pos] = item;
        } else {
            self.items.push(item);
        }
        self.save()
    }

    pub fn update_status(&mut self, id: &str, status: QueueStatus, error: Option<String>) -> Result<()> {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            item.status = status;
            item.error_message = error;
            self.save()?;
        }
        Ok(())
    }

    pub fn pause(&mut self, id: &str) -> Result<bool> {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            if item.status == QueueStatus::Downloading || item.status == QueueStatus::Pending {
                item.status = QueueStatus::Paused;
                self.save()?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn resume(&mut self, id: &str) -> Result<bool> {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            if item.status == QueueStatus::Paused || item.status == QueueStatus::Failed {
                item.status = QueueStatus::Pending;
                self.save()?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn clear_completed(&mut self) -> Result<()> {
        self.items.retain(|i| i.status != QueueStatus::Completed);
        self.save()
    }
}
