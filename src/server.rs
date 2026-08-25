use crate::downloader::Downloader;
use crate::library::{LibraryItem, LibraryStore};
use crate::models::{DownloadOptions, MangaFilter, OutputFormat, Page};
use crate::queue::{QueueItem, QueueStatus, QueueStore};
use crate::sources::json_source::JsonSource;
use crate::sources::mangadex::MangaDexSource;
use crate::sources::MangaSource;
use anyhow::Result;
use axum::{
    body::Body,
    extract::Query,
    http::{header, HeaderValue, StatusCode},
    response::{Html, Response},
    routing::{delete, get, post},
    Json, Router,
};
use indicatif::MultiProgress;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SourceInfo {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub languages: Vec<String>,
    pub is_nsfw: bool,
    pub tags: Vec<String>,
    pub icon_url: Option<String>,
}

#[derive(Serialize)]
pub struct DownloadedItemInfo {
    pub name: String,
    pub item_type: String,
    pub relative_path: String,
    pub size_formatted: String,
}

#[derive(Deserialize)]
pub struct OfflinePagesQuery {
    pub path: String,
}

#[derive(Deserialize)]
pub struct FilterQuery {
    pub source: Option<String>,
    pub q: Option<String>,
    pub genre: Option<String>,
    pub status: Option<String>,
    pub order: Option<String>,
    pub manga_type: Option<String>,
    pub demographic: Option<String>,
    pub nsfw: Option<bool>,
    pub page: Option<u32>,
    pub limit: Option<u32>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub source: Option<String>,
    pub q: String,
}

#[derive(Deserialize)]
pub struct LatestQuery {
    pub source: Option<String>,
}

#[derive(Deserialize)]
pub struct ChaptersQuery {
    pub source: Option<String>,
    pub id: String,
    pub lang: Option<String>,
}

#[derive(Deserialize)]
pub struct PagesQuery {
    pub source: Option<String>,
    pub id: String,
}

#[derive(Deserialize)]
pub struct ProxyQuery {
    pub url: String,
}

#[derive(Deserialize)]
pub struct RemoveLibraryQuery {
    pub id: String,
    pub source: Option<String>,
}

#[derive(Deserialize)]
pub struct QueueActionQuery {
    pub id: String,
}

#[derive(Deserialize)]
pub struct DownloadRequest {
    pub source: Option<String>,
    pub id: String,
    pub title: Option<String>,
    pub chapters: Option<String>,
    pub format: Option<String>,
    pub output_dir: Option<String>,
    pub cookies: Option<String>,
    pub user_agent: Option<String>,
    pub use_aria2: Option<bool>,
    pub compress: Option<bool>,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub success: bool,
    pub message: String,
}

pub fn get_source(name: Option<&str>) -> Box<dyn MangaSource> {
    let source_id = name.unwrap_or("mangadex").to_lowercase();

    let custom_dir = PathBuf::from("./custom_sources");
    if custom_dir.exists() {
        if let Ok(entries) = fs::read_dir(custom_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "json") {
                    if let Ok(js_src) = JsonSource::from_file(&path) {
                        if js_src.id().to_lowercase() == source_id {
                            return Box::new(js_src);
                        }
                    }
                }
            }
        }
    }

    Box::new(MangaDexSource::new())
}

async fn list_sources() -> Json<Vec<SourceInfo>> {
    let md = MangaDexSource::new();
    let mut list = vec![
        SourceInfo {
            id: md.id().to_string(),
            name: md.name().to_string(),
            base_url: md.base_url().to_string(),
            languages: md.languages(),
            is_nsfw: md.is_nsfw(),
            tags: md.tags(),
            icon_url: md.icon_url(),
        },
    ];

    let custom_dir = PathBuf::from("./custom_sources");
    if custom_dir.exists() {
        if let Ok(entries) = fs::read_dir(custom_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "json") {
                    if let Ok(js_src) = JsonSource::from_file(&path) {
                        list.push(SourceInfo {
                            id: js_src.id().to_string(),
                            name: js_src.name().to_string(),
                            base_url: js_src.base_url().to_string(),
                            languages: js_src.languages(),
                            is_nsfw: js_src.is_nsfw(),
                            tags: js_src.tags(),
                            icon_url: js_src.icon_url(),
                        });
                    }
                }
            }
        }
    }

    Json(list)
}

async fn list_offline_downloads() -> Json<Vec<DownloadedItemInfo>> {
    let mut items = Vec::new();
    let downloads_dir = PathBuf::from("./downloads");

    if downloads_dir.exists() {
        fn scan_dir(dir: &Path, items: &mut Vec<DownloadedItemInfo>) {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                    let rel_path = path.strip_prefix("./").unwrap_or(&path).to_string_lossy().to_string();

                    if path.is_file() {
                        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                        if ext == "cbz" || ext == "pdf" {
                            let metadata = fs::metadata(&path);
                            let size = metadata.map(|m| m.len()).unwrap_or(0);
                            let size_mb = format!("{:.2} MB", size as f64 / (1024.0 * 1024.0));

                            items.push(DownloadedItemInfo {
                                name,
                                item_type: ext.to_uppercase(),
                                relative_path: rel_path,
                                size_formatted: size_mb,
                            });
                        }
                    } else if path.is_dir() {
                        scan_dir(&path, items);
                    }
                }
            }
        }
        scan_dir(&downloads_dir, &mut items);
    }

    Json(items)
}

async fn get_offline_pages(Query(query): Query<OfflinePagesQuery>) -> Result<Json<Vec<Page>>, (StatusCode, String)> {
    let target_path = PathBuf::from(&query.path);
    if !target_path.exists() {
        return Err((StatusCode::NOT_FOUND, "File or directory not found".to_string()));
    }

    let mut pages = Vec::new();
    let ext = target_path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();

    if ext == "cbz" {
        let file = File::open(&target_path).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mut archive = ZipArchive::new(file).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

        let mut img_names = Vec::new();
        for i in 0..archive.len() {
            if let Ok(file) = archive.by_index(i) {
                let name = file.name().to_string();
                if name != "ComicInfo.xml" && (name.ends_with(".png") || name.ends_with(".jpg") || name.ends_with(".jpeg") || name.ends_with(".webp")) {
                    img_names.push(name);
                }
            }
        }
        img_names.sort();

        for (idx, img_name) in img_names.iter().enumerate() {
            if let Ok(mut file) = archive.by_name(img_name) {
                let mut buffer = Vec::new();
                if file.read_to_end(&mut buffer).is_ok() {
                    let mime = if img_name.ends_with(".webp") {
                        "image/webp"
                    } else if img_name.ends_with(".png") {
                        "image/png"
                    } else {
                        "image/jpeg"
                    };
                    let b64 = rfc4648_base64_encode(&buffer);
                    let data_url = format!("data:{};base64,{}", mime, b64);

                    pages.push(Page {
                        index: idx + 1,
                        filename: img_name.clone(),
                        url: data_url,
                    });
                }
            }
        }
    } else if target_path.is_dir() {
        if let Ok(entries) = fs::read_dir(&target_path) {
            let mut files: Vec<_> = entries
                .flatten()
                .filter(|e| e.path().is_file())
                .collect();
            files.sort_by_key(|e| e.path());

            for (idx, entry) in files.iter().enumerate() {
                let path = entry.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                if name != "ComicInfo.xml" {
                    if let Ok(buffer) = fs::read(&path) {
                        let mime = if name.ends_with(".webp") {
                            "image/webp"
                        } else if name.ends_with(".png") {
                            "image/png"
                        } else {
                            "image/jpeg"
                        };
                        let b64 = rfc4648_base64_encode(&buffer);
                        let data_url = format!("data:{};base64,{}", mime, b64);

                        pages.push(Page {
                            index: idx + 1,
                            filename: name,
                            url: data_url,
                        });
                    }
                }
            }
        }
    }

    Ok(Json(pages))
}

fn rfc4648_base64_encode(data: &[u8]) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(CHARSET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(CHARSET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARSET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARSET[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

async fn get_library() -> Json<Vec<LibraryItem>> {
    let store = LibraryStore::load();
    Json(store.items)
}

async fn add_to_library(Json(item): Json<LibraryItem>) -> Json<StatusResponse> {
    let mut store = LibraryStore::load();
    match store.add(item) {
        Ok(_) => Json(StatusResponse {
            success: true,
            message: "Manga added to library successfully".to_string(),
        }),
        Err(e) => Json(StatusResponse {
            success: false,
            message: e.to_string(),
        }),
    }
}

async fn remove_from_library(Query(query): Query<RemoveLibraryQuery>) -> Json<StatusResponse> {
    let mut store = LibraryStore::load();
    match store.remove(&query.id, query.source.as_deref()) {
        Ok(true) => Json(StatusResponse {
            success: true,
            message: "Removed from library".to_string(),
        }),
        _ => Json(StatusResponse {
            success: false,
            message: "Item not found in library".to_string(),
        }),
    }
}

async fn trigger_library_update() -> Json<StatusResponse> {
    tokio::spawn(async move {
        let mut store = LibraryStore::load();
        let _ = store
            .check_and_update_all(|src_id| get_source(Some(src_id)))
            .await;
    });

    Json(StatusResponse {
        success: true,
        message: "Library update process started in background".to_string(),
    })
}

async fn get_queue() -> Json<Vec<QueueItem>> {
    let store = QueueStore::load();
    Json(store.items)
}

async fn pause_queue_item(Query(query): Query<QueueActionQuery>) -> Json<StatusResponse> {
    let mut store = QueueStore::load();
    match store.pause(&query.id) {
        Ok(true) => Json(StatusResponse { success: true, message: "Paused download".to_string() }),
        _ => Json(StatusResponse { success: false, message: "Could not pause item".to_string() }),
    }
}

async fn resume_queue_item(Query(query): Query<QueueActionQuery>) -> Json<StatusResponse> {
    let mut store = QueueStore::load();
    match store.resume(&query.id) {
        Ok(true) => Json(StatusResponse { success: true, message: "Resumed download".to_string() }),
        _ => Json(StatusResponse { success: false, message: "Could not resume item".to_string() }),
    }
}

async fn search_manga(Query(query): Query<SearchQuery>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let source = get_source(query.source.as_deref());
    match source.search(&query.q).await {
        Ok(results) => Ok(Json(serde_json::to_value(results).unwrap())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn get_latest_manga(Query(query): Query<LatestQuery>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let source = get_source(query.source.as_deref());
    match source.get_latest().await {
        Ok(results) => Ok(Json(serde_json::to_value(results).unwrap())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn filter_manga_endpoint(Query(query): Query<FilterQuery>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let source = get_source(query.source.as_deref());
    let filter = MangaFilter {
        query: query.q.filter(|s| !s.trim().is_empty()),
        genre: query.genre.filter(|s| !s.trim().is_empty() && s != "all"),
        genres: None,
        status: query.status.filter(|s| !s.trim().is_empty() && s != "all"),
        order_by: query.order.filter(|s| !s.trim().is_empty()),
        manga_type: query.manga_type.filter(|s| !s.trim().is_empty() && s != "all"),
        demographic: query.demographic.filter(|s| !s.trim().is_empty() && s != "all"),
        language: None,
        is_nsfw: query.nsfw,
        page: query.page,
        limit: query.limit,
    };

    match source.filter_manga(&filter).await {
        Ok(results) => Ok(Json(serde_json::to_value(results).unwrap())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn get_genres_endpoint(Query(query): Query<LatestQuery>) -> Json<serde_json::Value> {
    let source = get_source(query.source.as_deref());
    let genres = source.available_genres();
    let sort_orders = source.available_sort_orders();
    Json(serde_json::json!({
        "genres": genres,
        "sort_orders": sort_orders
    }))
}

async fn get_manga_info(Query(query): Query<ChaptersQuery>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let source = get_source(query.source.as_deref());
    match source.get_manga_details(&query.id).await {
        Ok(Some(details)) => Ok(Json(serde_json::to_value(details).unwrap())),
        Ok(None) => Ok(Json(serde_json::Value::Null)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn get_chapters(Query(query): Query<ChaptersQuery>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let source = get_source(query.source.as_deref());
    match source.get_chapters(&query.id, query.lang.as_deref()).await {
        Ok(chapters) => Ok(Json(serde_json::to_value(chapters).unwrap())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn get_pages(Query(query): Query<PagesQuery>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let source = get_source(query.source.as_deref());
    match source.get_pages(&query.id).await {
        Ok(pages) => Ok(Json(serde_json::to_value(pages).unwrap())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn proxy_image(Query(query): Query<ProxyQuery>) -> Result<Response, (StatusCode, String)> {
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let res = client
        .get(&query.url)
        .header("Referer", &query.url)
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let content_type = res
        .headers()
        .get(header::CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("image/jpeg"));

    let bytes = res
        .bytes()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(header::CONTENT_TYPE, content_type);
    response
        .headers_mut()
        .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    Ok(response)
}

async fn download_manga(Json(req): Json<DownloadRequest>) -> Json<StatusResponse> {
    let source_name = req.source.clone();
    let manga_id = req.id.clone();
    let chapters_arg = req.chapters.unwrap_or_else(|| "all".to_string());
    let title_arg = req.title.unwrap_or_else(|| format!("Manga_{}", manga_id.replace('/', "_")));
    let output_path = PathBuf::from(req.output_dir.unwrap_or_else(|| "./downloads".to_string()));
    let use_aria = req.use_aria2.unwrap_or(false);
    let do_compress = req.compress.unwrap_or(false);
    let fmt = match req.format.as_deref() {
        Some("cbz") | Some("CBZ") => OutputFormat::Cbz,
        Some("pdf") | Some("PDF") => OutputFormat::Pdf,
        _ => OutputFormat::Folder,
    };

    tokio::spawn(async move {
        let source = get_source(source_name.as_deref());
        if let Ok(all_chapters) = source.get_chapters(&manga_id, None).await {
            let target_chapters: Vec<_> = if chapters_arg.to_lowercase() == "all" {
                all_chapters
            } else {
                all_chapters
                    .into_iter()
                    .filter(|c| c.chapter_number == chapters_arg)
                    .collect()
            };

            let options = DownloadOptions {
                output_dir: output_path,
                format: fmt.clone(),
                concurrent_downloads: 4,
                language: None,
                cookies: req.cookies,
                user_agent: req.user_agent,
                use_aria2: use_aria,
                compress_webp: do_compress,
            };

            let downloader = Downloader::new(options);
            let mp = MultiProgress::new();

            for ch in target_chapters {
                let task_id = format!("{}_{}_{}", source_name.as_deref().unwrap_or("src"), manga_id, ch.chapter_number);
                let mut qstore = QueueStore::load();
                let _ = qstore.push(QueueItem {
                    id: task_id.clone(),
                    source_id: source_name.as_deref().unwrap_or("mangadex").to_string(),
                    manga_id: manga_id.clone(),
                    manga_title: title_arg.clone(),
                    chapter_id: ch.id.clone(),
                    chapter_number: ch.chapter_number.clone(),
                    format: format!("{:?}", fmt),
                    output_dir: "./downloads".to_string(),
                    status: QueueStatus::Downloading,
                    downloaded_pages: 0,
                    total_pages: 0,
                    use_aria2: use_aria,
                    error_message: None,
                });

                match downloader.download_chapter(source.as_ref(), &title_arg, &ch, &mp).await {
                    Ok(_) => {
                        let _ = qstore.update_status(&task_id, QueueStatus::Completed, None);
                    }
                    Err(e) => {
                        let _ = qstore.update_status(&task_id, QueueStatus::Failed, Some(e.to_string()));
                    }
                }
            }
        }
    });

    Json(StatusResponse {
        success: true,
        message: if use_aria {
            "Download task started in background via aria2c accelerator".to_string()
        } else {
            "Download task started in background".to_string()
        },
    })
}

async fn web_app_index() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html lang="ar" dir="rtl">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Manga Source - القائمة الرئيسية والتصفح والقراءة</title>
    <link href="https://fonts.googleapis.com/css2?family=Cairo:wght@400;600;700;800;900&display=swap" rel="stylesheet">
    <style>
        :root {
            --bg-primary: #0b0f19;
            --bg-card: rgba(30, 41, 59, 0.75);
            --accent: #6366f1;
            --accent-glow: #818cf8;
            --text: #f8fafc;
            --text-secondary: #94a3b8;
            --border: rgba(255, 255, 255, 0.12);
            --gold: #f59e0b;
            --danger: #ef4444;
            --success: #10b981;
        }
        * { box-sizing: border-box; margin: 0; padding: 0; font-family: 'Cairo', sans-serif; }
        body { background: var(--bg-primary); color: var(--text); min-height: 100vh; display: flex; flex-direction: column; overflow-x: hidden; }

        header {
            position: sticky; top: 0; z-index: 100;
            background: rgba(11, 15, 25, 0.88); backdrop-filter: blur(16px);
            border-bottom: 1px solid var(--border); padding: 0.85rem 1.75rem;
            display: flex; justify-content: space-between; align-items: center; gap: 1rem; flex-wrap: wrap;
        }
        .brand { font-size: 1.45rem; font-weight: 800; background: linear-gradient(135deg, #6366f1, #a855f7); -webkit-background-clip: text; -webkit-text-fill-color: transparent; display: flex; align-items: center; gap: 0.5rem; cursor: pointer; }
        
        .controls-row { display: flex; gap: 0.75rem; align-items: center; flex-wrap: wrap; }
        select, input, button {
            background: #1e293b; color: var(--text); border: 1px solid var(--border);
            padding: 0.55rem 0.9rem; border-radius: 10px; font-size: 0.92rem; outline: none;
            transition: all 0.2s ease;
        }
        select:focus, input:focus { border-color: var(--accent); box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.25); }
        button { background: linear-gradient(135deg, #6366f1, #4f46e5); border: none; font-weight: 700; cursor: pointer; display: inline-flex; align-items: center; gap: 0.4rem; }
        button:hover { transform: translateY(-2px); box-shadow: 0 4px 15px rgba(99, 102, 241, 0.4); }

        .tabs { display: flex; gap: 0.5rem; }
        .tab-btn { background: transparent; border: 1px solid var(--border); color: var(--text-secondary); padding: 0.5rem 0.85rem; }
        .tab-btn.active { background: var(--accent); color: white; border-color: var(--accent); }

        main { flex: 1; max-width: 1400px; width: 100%; margin: 0 auto; padding: 1.75rem 1.25rem; display: flex; flex-direction: column; gap: 1.5rem; }
        
        .filter-banner {
            background: rgba(30, 41, 59, 0.5); border: 1px solid var(--border); border-radius: 14px;
            padding: 0.85rem 1.25rem; display: flex; justify-content: space-between; align-items: center; gap: 1rem; flex-wrap: wrap;
        }
        .filter-group { display: flex; align-items: center; gap: 0.75rem; flex-wrap: wrap; }
        .filter-label { font-size: 0.88rem; color: var(--text-secondary); font-weight: 600; }

        /* Discovery & Multi-Filter Control Hub */
        .discovery-panel {
            background: rgba(15, 23, 42, 0.6); border: 1px solid var(--border); border-radius: 16px;
            padding: 1.25rem; display: flex; flex-direction: column; gap: 1rem; box-shadow: 0 8px 25px rgba(0,0,0,0.3);
        }
        .search-box { display: flex; gap: 0.75rem; width: 100%; }
        .search-box input { flex: 1; font-size: 1.05rem; padding: 0.75rem 1.2rem; }

        .filters-grid {
            display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 0.75rem; align-items: flex-end;
        }
        .filter-field { display: flex; flex-direction: column; gap: 0.35rem; }
        .field-label { font-size: 0.82rem; color: #cbd5e1; font-weight: 700; }

        .quick-chips-wrapper {
            display: flex; gap: 0.45rem; overflow-x: auto; padding-bottom: 0.35rem; scrollbar-width: thin;
        }
        .quick-chip {
            background: rgba(30, 41, 59, 0.8); border: 1px solid var(--border); color: #cbd5e1;
            padding: 0.35rem 0.85rem; border-radius: 9999px; font-size: 0.82rem; font-weight: 600;
            white-space: nowrap; cursor: pointer; transition: all 0.2s;
        }
        .quick-chip:hover { border-color: var(--accent-glow); color: #fff; background: rgba(99, 102, 241, 0.2); }
        .quick-chip.active { background: var(--accent); color: #fff; border-color: var(--accent-glow); box-shadow: 0 0 10px rgba(99, 102, 241, 0.5); }

        .section-header { display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid var(--border); padding-bottom: 0.65rem; margin-bottom: 1rem; }
        .section-title { font-size: 1.25rem; font-weight: 800; color: var(--text); display: flex; align-items: center; gap: 0.5rem; }

        .manga-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: 1.5rem; }
        .manga-card {
            background: var(--bg-card); border-radius: 16px; border: 1px solid var(--border);
            display: flex; flex-direction: column; overflow: hidden;
            transition: all 0.3s ease; position: relative;
        }
        .manga-card:hover { transform: translateY(-6px); border-color: var(--accent); box-shadow: 0 16px 35px rgba(0, 0, 0, 0.6); }
        
        .cover-box { position: relative; width: 100%; height: 320px; background: #0f172a; overflow: hidden; }
        .cover-img { width: 100%; height: 100%; object-fit: cover; transition: transform 0.4s ease; }
        .manga-card:hover .cover-img { transform: scale(1.05); }

        /* Floating Badges on Cover */
        .badge-rating {
            position: absolute; top: 10px; right: 10px;
            background: linear-gradient(135deg, #f59e0b, #d97706); color: #000;
            font-weight: 800; font-size: 0.8rem; padding: 0.25rem 0.6rem; border-radius: 8px;
            box-shadow: 0 4px 10px rgba(0,0,0,0.5); z-index: 2; display: flex; align-items: center; gap: 0.25rem;
        }
        .badge-nsfw {
            position: absolute; top: 10px; left: 10px;
            background: linear-gradient(135deg, #ef4444, #be123c); color: #fff;
            font-weight: 800; font-size: 0.75rem; padding: 0.25rem 0.55rem; border-radius: 8px;
            box-shadow: 0 4px 10px rgba(0,0,0,0.5); z-index: 2;
        }
        .badge-type {
            position: absolute; bottom: 10px; left: 10px;
            background: rgba(15, 23, 42, 0.85); backdrop-filter: blur(8px);
            color: var(--accent-glow); font-size: 0.75rem; font-weight: 700;
            padding: 0.2rem 0.5rem; border-radius: 6px; border: 1px solid var(--border);
        }

        .card-details { padding: 1.1rem; display: flex; flex-direction: column; gap: 0.65rem; flex: 1; justify-content: space-between; }
        .manga-title { font-size: 1.05rem; font-weight: 700; line-height: 1.35; color: var(--text); display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
        .alt-title { font-size: 0.8rem; color: var(--text-secondary); margin-top: -0.2rem; }
        
        .meta-pills-row { display: flex; flex-wrap: wrap; gap: 0.35rem; align-items: center; }
        .meta-pill {
            font-size: 0.75rem; font-weight: 600; padding: 0.2rem 0.5rem; border-radius: 6px;
            background: rgba(15, 23, 42, 0.7); border: 1px solid var(--border); color: #cbd5e1;
            display: inline-flex; align-items: center; gap: 0.25rem;
        }
        .meta-pill.status-ongoing { color: #34d399; border-color: rgba(52, 211, 153, 0.3); background: rgba(52, 211, 153, 0.1); }
        .meta-pill.status-completed { color: #60a5fa; border-color: rgba(96, 165, 250, 0.3); background: rgba(96, 165, 250, 0.1); }
        .meta-pill.views-pill { color: #facc15; }
        .meta-pill.chapter-pill { color: #c084fc; font-weight: 700; }
        .meta-pill.time-pill { color: #94a3b8; }

        .chips-row { display: flex; flex-wrap: wrap; gap: 0.3rem; margin-top: 0.15rem; }
        .chip {
            font-size: 0.72rem; padding: 0.15rem 0.45rem; border-radius: 5px; cursor: pointer;
            background: rgba(99, 102, 241, 0.15); color: var(--accent-glow); border: 1px solid rgba(99, 102, 241, 0.25);
            transition: all 0.2s;
        }
        .chip:hover { background: var(--accent); color: #fff; }

        .manga-desc { font-size: 0.8rem; color: var(--text-secondary); line-height: 1.35; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }

        .card-actions { display: flex; gap: 0.5rem; margin-top: 0.5rem; }
        .card-actions button { flex: 1; padding: 0.5rem; font-size: 0.85rem; justify-content: center; }

        /* Modal Drawer */
        .modal-overlay {
            position: fixed; inset: 0; background: rgba(0, 0, 0, 0.85); backdrop-filter: blur(10px);
            display: none; justify-content: center; align-items: center; z-index: 200; padding: 1rem;
        }
        .modal-overlay.active { display: flex; }
        .modal-content {
            background: #131b2e; border: 1px solid var(--border); border-radius: 24px;
            width: 100%; max-width: 920px; max-height: 90vh; display: flex; flex-direction: column; overflow: hidden;
            box-shadow: 0 25px 60px rgba(0, 0, 0, 0.8);
        }
        .modal-header { padding: 1.1rem 1.5rem; border-bottom: 1px solid var(--border); display: flex; justify-content: space-between; align-items: center; background: rgba(15, 23, 42, 0.85); }
        .modal-body { padding: 1.5rem; overflow-y: auto; display: flex; flex-direction: column; gap: 1.25rem; }
        .close-btn { background: transparent; border: none; font-size: 1.6rem; color: var(--text-secondary); cursor: pointer; }
        
        /* Rich Manga Hero Header in Modal */
        .modal-hero {
            display: flex; gap: 1.5rem; background: rgba(30, 41, 59, 0.5); border: 1px solid var(--border);
            border-radius: 18px; padding: 1.25rem; flex-wrap: wrap; position: relative;
        }
        .hero-cover-wrap { width: 175px; height: 255px; border-radius: 12px; overflow: hidden; position: relative; flex-shrink: 0; background: #0f172a; }
        .hero-cover-img { width: 100%; height: 100%; object-fit: cover; }
        .hero-details { flex: 1; min-width: 280px; display: flex; flex-direction: column; gap: 0.55rem; }
        .hero-title { font-size: 1.35rem; font-weight: 800; color: #fff; line-height: 1.3; }
        .hero-alt { font-size: 0.85rem; color: var(--text-secondary); }
        .hero-meta-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 0.45rem; margin-top: 0.2rem; }
        .hero-meta-item { font-size: 0.82rem; background: rgba(15, 23, 42, 0.6); padding: 0.35rem 0.6rem; border-radius: 8px; border: 1px solid var(--border); display: flex; align-items: center; gap: 0.35rem; color: #e2e8f0; }
        
        .hero-synopsis {
            background: rgba(11, 15, 25, 0.6); border: 1px solid rgba(255, 255, 255, 0.08); border-radius: 10px;
            padding: 0.75rem 1rem; font-size: 0.83rem; color: #cbd5e1; line-height: 1.5; max-height: 110px; overflow-y: auto; margin-top: 0.2rem;
        }

        .hero-actions-bar { display: flex; gap: 0.5rem; flex-wrap: wrap; margin-top: 0.5rem; }
        .hero-actions-bar button { padding: 0.5rem 0.9rem; font-size: 0.85rem; }

        /* Chapters Toolbar & Rows */
        .chapters-toolbar { display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 0.75rem; border-bottom: 1px solid var(--border); padding-bottom: 0.65rem; }
        .chapters-toolbar h4 { font-size: 1.1rem; font-weight: 700; display: flex; align-items: center; gap: 0.4rem; }
        .chapters-toolbar input { width: 260px; padding: 0.45rem 0.85rem; font-size: 0.85rem; }

        .chapter-row {
            background: rgba(15, 23, 42, 0.65); padding: 0.75rem 1rem; border-radius: 10px; border: 1px solid rgba(255, 255, 255, 0.05);
            display: flex; justify-content: space-between; align-items: center; transition: all 0.2s; flex-wrap: wrap; gap: 0.5rem;
        }
        .chapter-row:hover { background: rgba(99, 102, 241, 0.18); border-color: var(--accent); }
        .ch-info { display: flex; flex-direction: column; gap: 0.2rem; }
        .ch-title { font-weight: 700; font-size: 0.95rem; color: #fff; }
        .ch-meta { font-size: 0.78rem; color: var(--text-secondary); }

        /* Reader Overlay */
        .reader-view {
            position: fixed; inset: 0; background: #070a12; z-index: 300;
            display: none; flex-direction: column; overflow-y: auto; align-items: center;
        }
        .reader-view.active { display: flex; }
        .reader-nav {
            position: sticky; top: 0; width: 100%; background: rgba(7, 10, 18, 0.92); backdrop-filter: blur(12px);
            border-bottom: 1px solid var(--border); padding: 0.85rem 1.75rem; display: flex; justify-content: space-between; align-items: center; z-index: 310;
        }
        .reader-body { width: 100%; max-width: 900px; padding: 2rem 1rem; display: flex; flex-direction: column; gap: 1rem; align-items: center; }
        .reader-img { width: 100%; border-radius: 8px; min-height: 250px; background: #1e293b; object-fit: contain; }

        .spinner { border: 3px solid rgba(255,255,255,0.1); border-top: 3px solid var(--accent); border-radius: 50%; width: 24px; height: 24px; animation: spin 1s linear infinite; }
        @keyframes spin { 0% { transform: rotate(0deg); } 100% { transform: rotate(360deg); } }
    </style>
</head>
<body>
    <header>
        <div class="brand" onclick="loadLatestManga()">🔥 Manga Source Web UI</div>
        <div class="controls-row">
            <div class="tabs">
                <button class="tab-btn active" id="tab-search" onclick="switchTab('search')">🏠 التصفح والاستكشاف</button>
                <button class="tab-btn" id="tab-library" onclick="switchTab('library')">📚 المكتبة</button>
                <button class="tab-btn" id="tab-offline" onclick="switchTab('offline')">📁 التنزيلات</button>
            </div>
            <label style="display: flex; align-items: center; gap: 0.35rem; font-size: 0.85rem; cursor: pointer; color: #10b981;">
                <input type="checkbox" id="compress-toggle" style="width: auto; cursor: pointer;"> 📦 ضغط WebP
            </label>
            <label style="display: flex; align-items: center; gap: 0.35rem; font-size: 0.85rem; cursor: pointer; color: var(--accent-glow);">
                <input type="checkbox" id="aria2-toggle" style="width: auto; cursor: pointer;"> 🚀 تسريع aria2c
            </label>
        </div>
    </header>

    <main>
        <div id="view-search">
            <!-- Filter Bar for Sources, Languages, and Tags -->
            <div class="filter-banner">
                <div class="filter-group">
                    <span class="filter-label">🌐 تصفية المصادر حسب اللغة:</span>
                    <select id="lang-filter" onchange="onFilterChange()">
                        <option value="all">الكل (All Languages)</option>
                        <option value="ar" selected>🇸🇦 العربية (Arabic)</option>
                        <option value="en">🇬🇧 English</option>
                    </select>

                    <label style="display: flex; align-items: center; gap: 0.35rem; font-size: 0.85rem; cursor: pointer; color: #f43f5e; margin-right: 0.5rem;">
                        <input type="checkbox" id="nsfw-toggle" onchange="onFilterChange()" style="width: auto; cursor: pointer;"> 🔞 إظهار مصادر ومحتوى +18
                    </label>
                </div>

                <div class="filter-group">
                    <span class="filter-label">📍 المصدر النشط:</span>
                    <select id="source-select" style="min-width: 220px; font-weight: 700;" onchange="onSourceChange()">
                        <!-- Filtered Dynamic Sources -->
                    </select>
                </div>
            </div>

            <!-- Modern Discovery & Advanced Multi-Filter Panel -->
            <div class="discovery-panel" style="margin-top: 1.25rem;">
                <div class="search-box">
                    <input type="text" id="search-input" placeholder="🔍 ابحث عن اسم المانجا أو الكلمة المفتاحية...">
                    <button id="search-btn" onclick="applyFilters()">🔍 بحث وتصفية</button>
                    <button style="background: #3b82f6;" onclick="resetFilters()">🔄 إعادة تعيين</button>
                </div>

                <div class="filters-grid">
                    <div class="filter-field">
                        <span class="field-label">🏷️ التصنيف والنوع (Genre):</span>
                        <select id="genre-select" onchange="applyFilters()">
                            <option value="all">الكل (جميع التصنيفات)</option>
                        </select>
                    </div>

                    <div class="filter-field">
                        <span class="field-label">🔄 الترتيب والفرز (Sort By):</span>
                        <select id="sort-select" onchange="applyFilters()">
                            <option value="latest">🔥 أحدث التحديثات (Latest)</option>
                            <option value="rating">⭐ الأعلى تقييماً (Rating)</option>
                            <option value="views">👁️ الأكثر مشاهدة وشعبية (Views)</option>
                            <option value="alphabet">🔤 أبجدي (A-Z / أ-ي)</option>
                            <option value="newest">🆕 الأحدث إضافة (Newest)</option>
                        </select>
                    </div>

                    <div class="filter-field">
                        <span class="field-label">🟢 حالة المانجا (Status):</span>
                        <select id="status-select" onchange="applyFilters()">
                            <option value="all">الكل (All Statuses)</option>
                            <option value="ongoing">🟢 مستمرة (Ongoing)</option>
                            <option value="completed">🔵 مكتملة (Completed)</option>
                            <option value="hiatus">⏸️ متوقفة مؤقتاً (Hiatus)</option>
                        </select>
                    </div>

                    <div class="filter-field">
                        <span class="field-label">📖 نوع العمل (Type):</span>
                        <select id="type-select" onchange="applyFilters()">
                            <option value="all">الكل (All Types)</option>
                            <option value="manga">🇯🇵 مانجا يابانية (Manga)</option>
                            <option value="manhwa">🇰🇷 مانهوا كورية (Manhwa)</option>
                            <option value="manhua">🇨🇳 مانها صينية (Manhua)</option>
                        </select>
                    </div>
                </div>

                <!-- Quick Genre Chips for One-Click Filtering -->
                <div class="quick-chips-wrapper" id="quick-genre-chips">
                    <!-- Populated dynamically -->
                </div>
            </div>

            <div class="section-header" style="margin-top: 1.75rem;">
                <div class="section-title" id="grid-header-title">🔥 أحدث المانجا والتحديثات المباشرة</div>
            </div>

            <div id="results-container" class="manga-grid">
                <!-- Results populated dynamically -->
            </div>
        </div>

        <div id="view-library" style="display: none;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem;">
                <h2>📚 المانجا المتابعة والمفضلة</h2>
                <button style="background: #10b981;" onclick="triggerLibraryUpdate()">⚡ فحص وتنزيل الفصول الجديدة</button>
            </div>
            <div id="library-container" class="manga-grid">
                <!-- Library items populated dynamically -->
            </div>
        </div>

        <div id="view-offline" style="display: none;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem;">
                <h2>📁 المعرض المحلي والقراءة بدون إنترنت (Offline Reader)</h2>
                <button onclick="loadOfflineDownloads()">🔄 تحديث القائمة</button>
            </div>
            <div id="offline-container" class="manga-grid">
                <!-- Offline items populated dynamically -->
            </div>
        </div>
    </main>

    <!-- Chapters & Rich Details Modal -->
    <div class="modal-overlay" id="chapters-modal">
        <div class="modal-content">
            <div class="modal-header">
                <h3 id="modal-manga-title">📖 تفاصيل المانجا وقائمة الفصول</h3>
                <button class="close-btn" onclick="closeModal()">&times;</button>
            </div>
            <div class="modal-body">
                <!-- Rich Header Area: Cover, Rating, Authors, Status, Year, Badges, Synopsis -->
                <div id="modal-manga-hero"></div>

                <!-- Chapters Section -->
                <div style="display: flex; flex-direction: column; gap: 0.75rem;">
                    <div class="chapters-toolbar">
                        <h4 id="modal-chapters-count">📚 قائمة الفصول المتاحة</h4>
                        <input type="text" id="chapter-filter-input" placeholder="🔍 تصفية الفصول بالرقم أو العنوان..." oninput="filterChaptersList()">
                    </div>
                    <div id="modal-chapters-list" style="display: flex; flex-direction: column; gap: 0.6rem;">
                        <!-- Chapters list rows -->
                    </div>
                </div>
            </div>
        </div>
    </div>

    <!-- Fullscreen Reader -->
    <div class="reader-view" id="reader-view">
        <div class="reader-nav">
            <h3 id="reader-title">قارئ الفصل</h3>
            <button onclick="closeReader()">❌ إغلاق القارئ</button>
        </div>
        <div class="reader-body" id="reader-pages">
            <!-- Pages rendered -->
        </div>
    </div>

    <script>
        const sourceSelect = document.getElementById('source-select');
        const langFilter = document.getElementById('lang-filter');
        const nsfwToggle = document.getElementById('nsfw-toggle');
        const searchInput = document.getElementById('search-input');
        const searchBtn = document.getElementById('search-btn');
        const genreSelect = document.getElementById('genre-select');
        const sortSelect = document.getElementById('sort-select');
        const statusSelect = document.getElementById('status-select');
        const typeSelect = document.getElementById('type-select');
        const quickGenreChips = document.getElementById('quick-genre-chips');
        const aria2Toggle = document.getElementById('aria2-toggle');
        const compressToggle = document.getElementById('compress-toggle');
        const resultsContainer = document.getElementById('results-container');
        const libraryContainer = document.getElementById('library-container');
        const offlineContainer = document.getElementById('offline-container');
        const gridHeaderTitle = document.getElementById('grid-header-title');
        const chaptersModal = document.getElementById('chapters-modal');
        const modalTitle = document.getElementById('modal-manga-title');
        const modalHero = document.getElementById('modal-manga-hero');
        const modalChaptersCount = document.getElementById('modal-chapters-count');
        const modalList = document.getElementById('modal-chapters-list');
        const chapterFilterInput = document.getElementById('chapter-filter-input');
        const readerView = document.getElementById('reader-view');
        const readerPages = document.getElementById('reader-pages');
        const readerTitle = document.getElementById('reader-title');

        let allSources = [];
        let currentTab = 'search';
        let currentChapters = [];
        let currentMangaId = '';
        let currentMangaTitle = '';
        let currentSource = '';
        let activeGenreId = 'all';

        function switchTab(tab) {
            currentTab = tab;
            document.getElementById('tab-search').classList.toggle('active', tab === 'search');
            document.getElementById('tab-library').classList.toggle('active', tab === 'library');
            document.getElementById('tab-offline').classList.toggle('active', tab === 'offline');
            document.getElementById('view-search').style.display = tab === 'search' ? 'block' : 'none';
            document.getElementById('view-library').style.display = tab === 'library' ? 'block' : 'none';
            document.getElementById('view-offline').style.display = tab === 'offline' ? 'block' : 'none';
            if (tab === 'library') loadLibrary();
            if (tab === 'offline') loadOfflineDownloads();
        }

        async function loadSourcesList() {
            try {
                const res = await fetch('/api/sources');
                allSources = await res.json();
                populateSourceDropdown();
                await onSourceChange();
            } catch (e) {
                console.error('Failed to load sources list', e);
            }
        }

        function populateSourceDropdown() {
            const selectedLang = langFilter.value;
            const allowNsfw = nsfwToggle.checked;
            const prevSelected = sourceSelect.value || '3asq';

            sourceSelect.innerHTML = '';
            
            const filtered = allSources.filter(src => {
                if (!allowNsfw && src.is_nsfw) return false;
                if (selectedLang !== 'all') {
                    if (!src.languages.includes(selectedLang) && !src.languages.includes('all')) return false;
                }
                return true;
            });

            filtered.forEach(src => {
                const opt = document.createElement('option');
                opt.value = src.id;
                const langBadge = src.languages.includes('ar') ? '🇸🇦' : (src.languages.includes('en') ? '🇬🇧' : '🌐');
                const nsfwBadge = src.is_nsfw ? '🔞' : '';
                opt.innerText = `${langBadge} ${src.name} ${nsfwBadge}`;
                if (src.id === prevSelected || (sourceSelect.children.length === 0 && src.id === '3asq')) {
                    opt.selected = true;
                }
                sourceSelect.appendChild(opt);
            });

            if (sourceSelect.options.length > 0 && !sourceSelect.value) {
                sourceSelect.selectedIndex = 0;
            }
        }

        async function onSourceChange() {
            await loadGenresForSource();
            applyFilters();
        }

        function onFilterChange() {
            populateSourceDropdown();
            onSourceChange();
        }

        async function loadGenresForSource() {
            const source = sourceSelect.value || '3asq';
            try {
                const res = await fetch(`/api/genres?source=${source}`);
                const data = await res.json();
                
                genreSelect.innerHTML = '<option value="all">الكل (جميع التصنيفات)</option>';
                if (data.genres) {
                    data.genres.forEach(g => {
                        const opt = document.createElement('option');
                        opt.value = g.id;
                        opt.innerText = g.name;
                        genreSelect.appendChild(opt);
                    });
                }

                // Quick Chips
                quickGenreChips.innerHTML = '';
                const allChip = document.createElement('button');
                allChip.className = 'quick-chip active';
                allChip.innerText = '🌐 الكل';
                allChip.onclick = () => selectQuickGenre('all', allChip);
                quickGenreChips.appendChild(allChip);

                if (data.genres) {
                    data.genres.slice(0, 15).forEach(g => {
                        const chip = document.createElement('button');
                        chip.className = 'quick-chip';
                        chip.innerText = g.name;
                        chip.onclick = () => selectQuickGenre(g.id, chip);
                        quickGenreChips.appendChild(chip);
                    });
                }

                if (data.sort_orders && data.sort_orders.length) {
                    const currentSort = sortSelect.value;
                    sortSelect.innerHTML = '';
                    data.sort_orders.forEach(s => {
                        const opt = document.createElement('option');
                        opt.value = s.id;
                        opt.innerText = s.name;
                        if (s.id === currentSort) opt.selected = true;
                        sortSelect.appendChild(opt);
                    });
                }
            } catch (e) {
                console.error('Failed to load genres', e);
            }
        }

        function selectQuickGenre(genreId, chipElement) {
            activeGenreId = genreId;
            genreSelect.value = genreId;
            document.querySelectorAll('.quick-chip').forEach(c => c.classList.remove('active'));
            if (chipElement) chipElement.classList.add('active');
            applyFilters();
        }

        function filterByGenre(genreName) {
            const opts = Array.from(genreSelect.options);
            const match = opts.find(o => o.value.toLowerCase() === genreName.toLowerCase() || o.text.toLowerCase().includes(genreName.toLowerCase()));
            if (match) {
                genreSelect.value = match.value;
            } else {
                genreSelect.value = genreName;
            }
            closeModal();
            applyFilters();
        }

        function resetFilters() {
            searchInput.value = '';
            genreSelect.value = 'all';
            sortSelect.value = 'latest';
            statusSelect.value = 'all';
            typeSelect.value = 'all';
            document.querySelectorAll('.quick-chip').forEach(c => c.classList.remove('active'));
            if (quickGenreChips.children[0]) quickGenreChips.children[0].classList.add('active');
            applyFilters();
        }

        async function applyFilters() {
            const source = sourceSelect.value || '3asq';
            const query = searchInput.value.trim();
            const genre = genreSelect.value;
            const order = sortSelect.value;
            const status = statusSelect.value;
            const mangaType = typeSelect.value;
            const nsfw = nsfwToggle.checked;

            let headerText = `🔥 استعراض المانجا (${sourceSelect.options[sourceSelect.selectedIndex]?.text || source})`;
            if (query) headerText = `🔍 بحث عن "${query}"`;
            if (genre && genre !== 'all') headerText += ` | تصنيف: ${genreSelect.options[genreSelect.selectedIndex]?.text || genre}`;
            if (order && order !== 'latest') headerText += ` | فرز: ${sortSelect.options[sortSelect.selectedIndex]?.text || order}`;
            gridHeaderTitle.innerText = headerText;

            resultsContainer.innerHTML = '<div style="grid-column: 1/-1; text-align: center; padding: 3rem;"><div class="spinner" style="margin: 0 auto 1rem;"></div>جاري تطبيق الفلاتر والفرز...</div>';

            const params = new URLSearchParams({
                source: source,
                q: query,
                genre: genre,
                order: order,
                status: status,
                manga_type: mangaType,
                nsfw: nsfw
            });

            try {
                const res = await fetch(`/api/filter?${params.toString()}`);
                const data = await res.json();
                renderMangaGrid(data);
            } catch (e) {
                resultsContainer.innerHTML = `<div style="grid-column: 1/-1; text-align: center; color: #f87171;">خطأ أثناء جلب النتائج: ${e.message}</div>`;
            }
        }

        function loadLatestManga() {
            resetFilters();
        }

        function renderMangaGrid(data) {
            resultsContainer.innerHTML = '';
            if (!data || data.length === 0) {
                resultsContainer.innerHTML = '<div style="grid-column: 1/-1; text-align: center; padding: 3rem; color: var(--text-secondary);">لم يتم العثور على نتائج.</div>';
                return;
            }

            data.forEach(manga => {
                const coverUrl = manga.cover_url ? `/api/proxy?url=${encodeURIComponent(manga.cover_url)}` : 'https://via.placeholder.com/300x400?text=No+Cover';
                const card = document.createElement('div');
                card.className = 'manga-card';

                const isOngoing = manga.status && (manga.status.includes('مستمر') || manga.status.toLowerCase().includes('ongoing'));
                const isCompleted = manga.status && (manga.status.includes('مكتمل') || manga.status.toLowerCase().includes('completed'));
                const statusClass = isOngoing ? 'status-ongoing' : (isCompleted ? 'status-completed' : '');

                card.innerHTML = `
                    <div class="cover-box" style="cursor: pointer;" onclick="fetchChapters('${manga.id}', '${manga.title.replace(/'/g, "\\'")}')">
                        <img class="cover-img" src="${coverUrl}" alt="${manga.title}" loading="lazy">
                        ${manga.rating ? `<div class="badge-rating">⭐ ${Number(manga.rating).toFixed(1)}</div>` : ''}
                        ${manga.is_nsfw ? `<div class="badge-nsfw">🔞 18+</div>` : ''}
                        ${manga.manga_type ? `<div class="badge-type">${manga.manga_type}</div>` : ''}
                    </div>
                    <div class="card-details">
                        <div style="cursor: pointer;" onclick="fetchChapters('${manga.id}', '${manga.title.replace(/'/g, "\\'")}')">
                            <h3 class="manga-title" title="${manga.title}">${manga.title}</h3>
                            ${manga.alt_titles && manga.alt_titles.length ? `<div class="alt-title">${manga.alt_titles[0]}</div>` : ''}
                            ${manga.author ? `<div style="font-size: 0.8rem; color: var(--accent-glow); margin-top: 0.2rem;">👤 ${manga.author}</div>` : ''}
                        </div>

                        <div class="meta-pills-row">
                            ${manga.status ? `<span class="meta-pill ${statusClass}">● ${manga.status}</span>` : ''}
                            ${manga.latest_chapter ? `<span class="meta-pill chapter-pill">📌 ف.${manga.latest_chapter}</span>` : ''}
                            ${manga.views ? `<span class="meta-pill views-pill">👁️ ${manga.views}</span>` : ''}
                            ${manga.updated_at ? `<span class="meta-pill time-pill">🕒 ${manga.updated_at}</span>` : ''}
                        </div>

                        ${manga.genres && manga.genres.length ? `
                            <div class="chips-row">
                                ${manga.genres.slice(0, 4).map(g => `<span class="chip" title="تصفية حسب ${g}" onclick="event.stopPropagation(); filterByGenre('${g.replace(/'/g, "\\'")}')">${g}</span>`).join('')}
                            </div>
                        ` : ''}

                        ${manga.tags && manga.tags.length ? `
                            <div class="chips-row">
                                ${manga.tags.slice(0, 3).map(t => `<span class="chip" style="background: rgba(245, 158, 11, 0.15); color: #f59e0b; border-color: rgba(245, 158, 11, 0.3);">${t}</span>`).join('')}
                            </div>
                        ` : ''}

                        ${manga.description ? `<p class="manga-desc">${manga.description}</p>` : ''}

                        <div class="card-actions">
                            <button onclick="fetchChapters('${manga.id}', '${manga.title.replace(/'/g, "\\'")}')">📖 التفاصيل والفصول</button>
                            <button style="background: #a855f7;" onclick="addToLibrary('${manga.id}', '${manga.title.replace(/'/g, "\\'")}', '${manga.cover_url || ''}')">📌 للمكتبة</button>
                        </div>
                    </div>
                `;
                resultsContainer.appendChild(card);
            });
        }

        async function loadOfflineDownloads() {
            offlineContainer.innerHTML = '<div style="grid-column: 1/-1; text-align: center; padding: 3rem;"><div class="spinner" style="margin: 0 auto 1rem;"></div>جاري فحص الفصول والملفات المنزلة محلياً...</div>';
            try {
                const res = await fetch('/api/downloads/list');
                const items = await res.json();

                offlineContainer.innerHTML = '';
                if (!items || items.length === 0) {
                    offlineContainer.innerHTML = '<div style="grid-column: 1/-1; text-align: center; padding: 3rem; color: var(--text-secondary);">لا توجد ملفات مانجا منزلة حالياً في مجلد ./downloads/</div>';
                    return;
                }

                items.forEach(item => {
                    const card = document.createElement('div');
                    card.className = 'manga-card';
                    card.innerHTML = `
                        <div class="cover-box" style="display: flex; justify-content: center; align-items: center; background: #1e293b;">
                            <div style="font-size: 3.5rem;">${item.item_type === 'CBZ' ? '📦' : '📄'}</div>
                        </div>
                        <div class="card-details">
                            <div>
                                <h3 class="manga-title" style="word-break: break-word;">${item.name}</h3>
                                <div class="author-tag">💾 الحجم: ${item.size_formatted}</div>
                                <div style="font-size: 0.82rem; color: var(--text-secondary); margin-top: 0.25rem;">الصيغة: ${item.item_type}</div>
                            </div>
                            <div class="card-actions">
                                <button onclick="openOfflineReader('${item.relative_path.replace(/'/g, "\\'")}', '${item.name.replace(/'/g, "\\'")}')">📖 قراءة أوفلاين</button>
                            </div>
                        </div>
                    `;
                    offlineContainer.appendChild(card);
                });
            } catch (e) {
                offlineContainer.innerHTML = `<div style="grid-column: 1/-1; text-align: center; color: #f87171;">خطأ في قراءة المعرض المحلي: ${e.message}</div>`;
            }
        }

        async function openOfflineReader(relPath, title) {
            readerTitle.innerText = `قراءة أوفلاين: ${title}`;
            readerPages.innerHTML = '<div style="padding: 3rem; text-align: center;"><div class="spinner" style="margin: 0 auto 1rem;"></div>جاري قراءة الصفحات من الملف المحلي...</div>';
            readerView.classList.add('active');

            try {
                const res = await fetch(`/api/downloads/pages?path=${encodeURIComponent(relPath)}`);
                const pages = await res.json();

                readerPages.innerHTML = '';
                if (!pages || pages.length === 0) {
                    readerPages.innerHTML = '<div style="padding: 2rem; color: #f87171;">تعذر قراءة صفحات هذا الملف المحلي.</div>';
                    return;
                }

                pages.forEach((page, idx) => {
                    const img = document.createElement('img');
                    img.className = 'reader-img';
                    img.loading = 'lazy';
                    img.alt = `صفحة ${idx + 1}`;
                    img.src = page.url;
                    readerPages.appendChild(img);
                });
            } catch (e) {
                readerPages.innerHTML = `<div style="padding: 2rem; color: #f87171;">خطأ أثناء قراءة الملف: ${e.message}</div>`;
            }
        }

        async function loadLibrary() {
            libraryContainer.innerHTML = '<div style="grid-column: 1/-1; text-align: center; padding: 3rem;"><div class="spinner" style="margin: 0 auto 1rem;"></div>جاري تحميل المكتبة الشخصية...</div>';
            try {
                const res = await fetch('/api/library');
                const items = await res.json();

                libraryContainer.innerHTML = '';
                if (!items || items.length === 0) {
                    libraryContainer.innerHTML = '<div style="grid-column: 1/-1; text-align: center; padding: 3rem; color: var(--text-secondary);">المكتبة فارغة حالياً. قم بإضافة مانجا من تبويب البحث!</div>';
                    return;
                }

                items.forEach(item => {
                    const coverUrl = item.cover_url ? `/api/proxy?url=${encodeURIComponent(item.cover_url)}` : 'https://via.placeholder.com/300x400?text=No+Cover';
                    const card = document.createElement('div');
                    card.className = 'manga-card';
                    card.innerHTML = `
                        <div class="cover-box">
                            <img class="cover-img" src="${coverUrl}" alt="${item.title}" loading="lazy">
                        </div>
                        <div class="card-details">
                            <div>
                                <h3 class="manga-title">${item.title}</h3>
                                <div class="author-tag">📍 المصدر: ${item.source_id}</div>
                                <div style="font-size: 0.85rem; color: #10b981; margin-top: 0.25rem;">📌 آخر فصل منزل: ${item.last_downloaded_chapter || 'لم يبدأ'}</div>
                            </div>
                            <div class="card-actions">
                                <button onclick="fetchChaptersForSource('${item.manga_id}', '${item.title.replace(/'/g, "\\'")}', '${item.source_id}')">📖 التفاصيل والفصول</button>
                                <button style="background: #ef4444;" onclick="removeFromLibrary('${item.manga_id}', '${item.source_id}')">🗑️ إزالة</button>
                            </div>
                        </div>
                    `;
                    libraryContainer.appendChild(card);
                });
            } catch (e) {
                libraryContainer.innerHTML = `<div style="grid-column: 1/-1; text-align: center; color: #f87171;">خطأ في تحميل المكتبة: ${e.message}</div>`;
            }
        }

        async function addToLibrary(mangaId, title, coverUrl) {
            const source = sourceSelect.value;
            try {
                const res = await fetch('/api/library/add', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({
                        manga_id: mangaId,
                        source_id: source,
                        title: title,
                        cover_url: coverUrl || null,
                        last_downloaded_chapter: null,
                        preferred_format: 'cbz',
                        output_dir: './downloads'
                    })
                });
                const data = await res.json();
                alert(` تم إضافة "${title}" إلى مكتبتك الشخصية!`);
            } catch (e) {
                alert('خطأ أثناء الإضافة للمكتبة: ' + e.message);
            }
        }

        async function removeFromLibrary(mangaId, sourceId) {
            if (!confirm('هل أنت تأكد من إزالة المانجا من مكتبتك الشخصية؟')) return;
            try {
                await fetch(`/api/library/remove?id=${encodeURIComponent(mangaId)}&source=${encodeURIComponent(sourceId)}`, { method: 'DELETE' });
                loadLibrary();
            } catch (e) {
                alert('خطأ أثناء الحذف: ' + e.message);
            }
        }

        async function triggerLibraryUpdate() {
            try {
                const res = await fetch('/api/library/update', { method: 'POST' });
                const data = await res.json();
                alert('⚡ تم بدء فحص وتنزيل الفصول الجديدة في الخلفية بنجاح!');
            } catch (e) {
                alert('خطأ: ' + e.message);
            }
        }

        function fetchChapters(mangaId, mangaTitle) {
            fetchChaptersForSource(mangaId, mangaTitle, sourceSelect.value);
        }

        async function fetchChaptersForSource(mangaId, mangaTitle, source) {
            currentMangaId = mangaId;
            currentMangaTitle = mangaTitle;
            currentSource = source;
            chapterFilterInput.value = '';

            modalTitle.innerText = `📖 ${mangaTitle}`;
            modalHero.innerHTML = '<div style="width: 100%; text-align: center; padding: 1.5rem;"><div class="spinner" style="margin: 0 auto 0.75rem;"></div>جاري جلب كل التفاصيل الكاملة للمانجا...</div>';
            modalList.innerHTML = '<div style="text-align: center; padding: 2rem;"><div class="spinner" style="margin: 0 auto 1rem;"></div>جاري استخراج قائمة الفصول...</div>';
            modalChaptersCount.innerText = '📚 قائمة الفصول';
            chaptersModal.classList.add('active');

            try {
                const [infoRes, chRes] = await Promise.all([
                    fetch(`/api/manga?source=${source}&id=${encodeURIComponent(mangaId)}`).catch(() => null),
                    fetch(`/api/chapters?source=${source}&id=${encodeURIComponent(mangaId)}`).catch(() => null)
                ]);

                const mangaInfo = infoRes ? await infoRes.json() : null;
                const chapters = chRes ? await chRes.json() : [];
                currentChapters = chapters || [];

                // Render Rich Hero Section
                renderModalHero(mangaInfo, mangaId, mangaTitle, source);

                // Render Chapters List
                renderChaptersList(currentChapters);

            } catch (e) {
                modalHero.innerHTML = `<div style="color: #f87171; text-align: center; padding: 1rem;">خطأ في جلب التفاصيل: ${e.message}</div>`;
                modalList.innerHTML = `<div style="color: #f87171; text-align: center; padding: 1rem;">خطأ في جلب الفصول: ${e.message}</div>`;
            }
        }

        function renderModalHero(manga, mangaId, mangaTitle, source) {
            const title = (manga && manga.title) || mangaTitle;
            const coverUrl = (manga && manga.cover_url) ? `/api/proxy?url=${encodeURIComponent(manga.cover_url)}` : 'https://via.placeholder.com/300x400?text=No+Cover';
            const ratingText = (manga && manga.rating) ? `⭐ ${Number(manga.rating).toFixed(1)} / 5 ${manga.rating_count ? '(' + manga.rating_count + ' تصويت)' : ''}` : 'غير مقيم';

            const firstChapter = currentChapters.length > 0 ? currentChapters[0] : null;
            const latestChapter = currentChapters.length > 0 ? currentChapters[currentChapters.length - 1] : null;

            modalHero.innerHTML = `
                <div class="hero-cover-wrap">
                    <img class="hero-cover-img" src="${coverUrl}" alt="${title}">
                    ${(manga && manga.rating) ? `<div class="badge-rating">⭐ ${Number(manga.rating).toFixed(1)}</div>` : ''}
                    ${(manga && manga.is_nsfw) ? `<div class="badge-nsfw">🔞 18+</div>` : ''}
                    ${(manga && manga.manga_type) ? `<div class="badge-type">${manga.manga_type}</div>` : ''}
                </div>
                <div class="hero-details">
                    <h2 class="hero-title">${title}</h2>
                    ${(manga && manga.alt_titles && manga.alt_titles.length) ? `<div class="hero-alt">الأسماء البديلة: ${manga.alt_titles.join(' | ')}</div>` : ''}

                    <div class="hero-meta-grid">
                        ${(manga && manga.author) ? `<div class="hero-meta-item">👤 الكاتب: <strong>${manga.author}</strong></div>` : ''}
                        ${(manga && manga.artist) ? `<div class="hero-meta-item">🎨 الرسام: <strong>${manga.artist}</strong></div>` : ''}
                        ${(manga && manga.status) ? `<div class="hero-meta-item">🟢 الحالة: <strong>${manga.status}</strong></div>` : ''}
                        ${(manga && manga.release_year) ? `<div class="hero-meta-item">📅 سنة الإصدار: <strong>${manga.release_year}</strong></div>` : ''}
                        ${(manga && manga.views) ? `<div class="hero-meta-item">👁️ المشاهدات: <strong>${manga.views}</strong></div>` : ''}
                        <div class="hero-meta-item">⭐ التقييم: <strong>${ratingText}</strong></div>
                    </div>

                    ${(manga && manga.genres && manga.genres.length) ? `
                        <div class="chips-row" style="margin-top: 0.35rem;">
                            ${manga.genres.map(g => `<span class="chip" title="تصفية حسب ${g}" onclick="filterByGenre('${g.replace(/'/g, "\\'")}')">${g}</span>`).join('')}
                        </div>
                    ` : ''}

                    ${(manga && manga.tags && manga.tags.length) ? `
                        <div class="chips-row">
                            ${manga.tags.map(t => `<span class="chip" style="background: rgba(245, 158, 11, 0.15); color: #f59e0b; border-color: rgba(245, 158, 11, 0.3);">${t}</span>`).join('')}
                        </div>
                    ` : ''}

                    ${(manga && manga.description) ? `
                        <div class="hero-synopsis">
                            <strong>القصة والنبذة:</strong> ${manga.description}
                        </div>
                    ` : ''}

                    <div class="hero-actions-bar">
                        <button style="background: #a855f7;" onclick="addToLibrary('${mangaId}', '${title.replace(/'/g, "\\'")}', '${(manga && manga.cover_url) || ''}')">📌 إضافة للمكتبة</button>
                        ${firstChapter ? `<button style="background: #3b82f6;" onclick="openReader('${firstChapter.id}', 'فصل ${firstChapter.chapter_number}', '${source}')">⚡ قراءة أول فصل (${firstChapter.chapter_number})</button>` : ''}
                        ${latestChapter && latestChapter !== firstChapter ? `<button style="background: #ec4899;" onclick="openReader('${latestChapter.id}', 'فصل ${latestChapter.chapter_number}', '${source}')">🔥 قراءة أحدث فصل (${latestChapter.chapter_number})</button>` : ''}
                    </div>
                </div>
            `;
        }

        function filterChaptersList() {
            const filterText = chapterFilterInput.value.trim().toLowerCase();
            if (!filterText) {
                renderChaptersList(currentChapters);
                return;
            }

            const filtered = currentChapters.filter(ch => {
                const numMatch = ch.chapter_number && ch.chapter_number.toLowerCase().includes(filterText);
                const titleMatch = ch.title && ch.title.toLowerCase().includes(filterText);
                return numMatch || titleMatch;
            });

            renderChaptersList(filtered);
        }

        function renderChaptersList(chapters) {
            modalChaptersCount.innerText = `📚 قائمة الفصول (${chapters.length} فصل)`;
            modalList.innerHTML = '';

            if (!chapters || chapters.length === 0) {
                modalList.innerHTML = '<div style="text-align: center; padding: 2rem; color: var(--text-secondary);">لا توجد فصول مطابقة.</div>';
                return;
            }

            chapters.forEach(ch => {
                const row = document.createElement('div');
                row.className = 'chapter-row';
                row.innerHTML = `
                    <div class="ch-info">
                        <div class="ch-title">فصل ${ch.chapter_number} ${ch.title ? '- ' + ch.title : ''}</div>
                        ${ch.release_date ? `<div class="ch-meta">🕒 تاريخ النشر: ${ch.release_date}</div>` : ''}
                    </div>
                    <div style="display: flex; gap: 0.4rem; align-items: center;">
                        <button onclick="openReader('${ch.id}', 'فصل ${ch.chapter_number}', '${currentSource}')">👁️ قراءة</button>
                        <button style="background: #10b981;" onclick="triggerDownload('${currentMangaId}', '${currentMangaTitle.replace(/'/g, "\\'")}', '${ch.chapter_number}', 'cbz', '${currentSource}')">⬇️ CBZ</button>
                        <button style="background: #ef4444;" onclick="triggerDownload('${currentMangaId}', '${currentMangaTitle.replace(/'/g, "\\'")}', '${ch.chapter_number}', 'pdf', '${currentSource}')">📄 PDF</button>
                    </div>
                `;
                modalList.appendChild(row);
            });
        }

        async function openReader(chapterId, chTitle, srcId) {
            const source = srcId || sourceSelect.value;
            readerTitle.innerText = `${chTitle} (${source})`;
            readerPages.innerHTML = '<div style="padding: 3rem; text-align: center;"><div class="spinner" style="margin: 0 auto 1rem;"></div>جاري تحميل صور الفصل المباشرة...</div>';
            readerView.classList.add('active');

            try {
                const res = await fetch(`/api/pages?source=${source}&id=${encodeURIComponent(chapterId)}`);
                const pages = await res.json();

                readerPages.innerHTML = '';
                if (!pages || pages.length === 0) {
                    readerPages.innerHTML = '<div style="padding: 2rem; color: #f87171;">فشل جلب صفحات هذا الفصل.</div>';
                    return;
                }

                pages.forEach((page, idx) => {
                    const img = document.createElement('img');
                    img.className = 'reader-img';
                    img.loading = 'lazy';
                    img.alt = `صفحة ${idx + 1}`;
                    img.src = `/api/proxy?url=${encodeURIComponent(page.url)}`;
                    readerPages.appendChild(img);
                });
            } catch (e) {
                readerPages.innerHTML = `<div style="padding: 2rem; color: #f87171;">خطأ: ${e.message}</div>`;
            }
        }

        async function triggerDownload(mangaId, title, chNum, fmt, srcId) {
            const source = srcId || sourceSelect.value;
            const formatName = (fmt || 'cbz').toUpperCase();
            const useAria = aria2Toggle.checked;
            const doCompress = compressToggle.checked;
            try {
                const res = await fetch('/api/download', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({
                        source: source,
                        id: mangaId,
                        title: title,
                        chapters: chNum,
                        format: fmt || 'cbz',
                        use_aria2: useAria,
                        compress: doCompress
                    })
                });
                const data = await res.json();
                alert(` ${data.message}`);
            } catch (e) {
                alert('خطأ في إرسال طلب التنزيل: ' + e.message);
            }
        }

        function closeModal() { chaptersModal.classList.remove('active'); }
        function closeReader() { readerView.classList.remove('active'); }

        searchBtn.addEventListener('click', performSearch);
        searchInput.addEventListener('keypress', e => { if (e.key === 'Enter') performSearch(); });
        sourceSelect.addEventListener('change', loadLatestManga);
        
        loadSourcesList();
    </script>
</body>
</html>"#)
}

async fn reader_ui(Query(query): Query<PagesQuery>) -> Html<String> {
    let source_id = query.source.unwrap_or_else(|| "mangadex".to_string());
    let chapter_id = query.id;

    let html_content = format!(
        r#"<!DOCTYPE html>
<html lang="ar" dir="rtl">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>قارئ المانجا المباشر - {ch_id}</title>
    <link href="https://fonts.googleapis.com/css2?family=Cairo:wght@400;600;700&display=swap" rel="stylesheet">
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; font-family: 'Cairo', sans-serif; }}
        body {{ background: #0f172a; color: #f8fafc; display: flex; flex-direction: column; align-items: center; min-height: 100vh; }}
        header {{ position: sticky; top: 0; width: 100%; background: rgba(15, 23, 42, 0.9); backdrop-filter: blur(12px); border-bottom: 1px solid rgba(255,255,255,0.1); padding: 1rem 2rem; display: flex; justify-content: space-between; align-items: center; z-index: 100; }}
        h1 {{ font-size: 1.25rem; font-weight: 700; background: linear-gradient(135deg, #6366f1, #a855f7); -webkit-background-clip: text; -webkit-text-fill-color: transparent; }}
        .badge {{ background: rgba(99, 102, 241, 0.2); color: #818cf8; padding: 0.25rem 0.75rem; border-radius: 9999px; font-size: 0.85rem; font-weight: 600; }}
        #reader-container {{ display: flex; flex-direction: column; align-items: center; gap: 1rem; width: 100%; max-width: 900px; padding: 2rem 1rem; }}
        .page-wrapper {{ position: relative; width: 100%; display: flex; justify-content: center; background: #1e293b; border-radius: 12px; overflow: hidden; box-shadow: 0 10px 25px -5px rgba(0, 0, 0, 0.5); }}
        .page-img {{ width: 100%; height: auto; display: block; object-fit: contain; min-height: 200px; border-radius: 8px; }}
        .loading {{ padding: 3rem; text-align: center; color: #94a3b8; font-size: 1.2rem; }}
        .error {{ padding: 2rem; color: #f87171; background: rgba(239, 68, 68, 0.1); border-radius: 8px; text-align: center; }}
    </style>
</head>
<body>
    <header>
        <h1>📖 قارئ المانجا المباشر</h1>
        <div class="badge">المصدر: {src_id} | الفصل: {ch_id}</div>
    </header>

    <div id="reader-container">
        <div class="loading" id="loader">⚡ جاري تحميل صفحات الفصل...</div>
    </div>

    <script>
        async function loadChapterPages() {{
            const container = document.getElementById('reader-container');
            try {{
                const res = await fetch(`/api/pages?source={src_id}&id={ch_id}`);
                if (!res.ok) throw new Error('فشل جلب الصفحات');
                const pages = await res.json();
                
                container.innerHTML = '';
                if (!pages || pages.length === 0) {{
                    container.innerHTML = '<div class="error">لم يتم العثور على صفحات لهذا الفصل.</div>';
                    return;
                }}

                pages.forEach((page, idx) => {{
                    const wrapper = document.createElement('div');
                    wrapper.className = 'page-wrapper';
                    
                    const img = document.createElement('img');
                    img.className = 'page-img';
                    img.loading = 'lazy';
                    img.alt = `صفحة ${{idx + 1}}`;
                    img.src = `/api/proxy?url=${{encodeURIComponent(page.url)}}`;
                    
                    wrapper.appendChild(img);
                    container.appendChild(wrapper);
                }});
            }} catch (err) {{
                container.innerHTML = `<div class="error">حدث خطأ أثناء تحميل الفصل: ${{err.message}}</div>`;
            }}
        }}
        loadChapterPages();
    </script>
</body>
</html>"#,
        src_id = source_id,
        ch_id = chapter_id
    );

    Html(html_content)
}

pub async fn start_server(port: u16) -> Result<()> {
    let app = Router::new()
        .route("/", get(web_app_index))
        .route("/api/sources", get(list_sources))
        .route("/api/search", get(search_manga))
        .route("/api/latest", get(get_latest_manga))
        .route("/api/filter", get(filter_manga_endpoint))
        .route("/api/genres", get(get_genres_endpoint))
        .route("/api/manga", get(get_manga_info))
        .route("/api/chapters", get(get_chapters))
        .route("/api/pages", get(get_pages))
        .route("/api/proxy", get(proxy_image))
        .route("/api/download", post(download_manga))
        .route("/api/library", get(get_library))
        .route("/api/library/add", post(add_to_library))
        .route("/api/library/remove", delete(remove_from_library))
        .route("/api/library/update", post(trigger_library_update))
        .route("/api/downloads/list", get(list_offline_downloads))
        .route("/api/downloads/pages", get(get_offline_pages))
        .route("/api/queue", get(get_queue))
        .route("/api/queue/pause", post(pause_queue_item))
        .route("/api/queue/resume", post(resume_queue_item))
        .route("/reader", get(reader_ui));

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("Manga Source Web App Dashboard running on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
