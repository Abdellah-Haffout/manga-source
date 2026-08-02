use crate::downloader::Downloader;
use crate::library::{LibraryItem, LibraryStore};
use crate::models::{DownloadOptions, OutputFormat, Page};
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

#[derive(Serialize)]
pub struct SourceInfo {
    pub id: String,
    pub name: String,
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
pub struct SearchQuery {
    pub source: Option<String>,
    pub q: String,
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
    let mut list = vec![
        SourceInfo { id: "mangadex".to_string(), name: "MangaDex (Global Multi-Language API)".to_string() },
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
    <title>Manga Source - تطبيق القراءة والمكتبة وقارئ الأوفلاين</title>
    <link href="https://fonts.googleapis.com/css2?family=Cairo:wght@400;600;700;800&display=swap" rel="stylesheet">
    <style>
        :root {
            --bg-primary: #0b0f19;
            --bg-card: rgba(30, 41, 59, 0.75);
            --accent: #6366f1;
            --accent-glow: #818cf8;
            --text: #f8fafc;
            --text-secondary: #94a3b8;
            --border: rgba(255, 255, 255, 0.12);
        }
        * { box-sizing: border-box; margin: 0; padding: 0; font-family: 'Cairo', sans-serif; }
        body { background: var(--bg-primary); color: var(--text); min-height: 100vh; display: flex; flex-direction: column; overflow-x: hidden; }

        header {
            position: sticky; top: 0; z-index: 100;
            background: rgba(11, 15, 25, 0.85); backdrop-filter: blur(16px);
            border-bottom: 1px solid var(--border); padding: 1rem 2rem;
            display: flex; justify-content: space-between; align-items: center; gap: 1rem;
        }
        .brand { font-size: 1.5rem; font-weight: 800; background: linear-gradient(135deg, #6366f1, #a855f7); -webkit-background-clip: text; -webkit-text-fill-color: transparent; display: flex; align-items: center; gap: 0.5rem; }
        
        .controls-row { display: flex; gap: 1rem; align-items: center; flex-wrap: wrap; }
        select, input, button {
            background: #1e293b; color: var(--text); border: 1px solid var(--border);
            padding: 0.6rem 1rem; border-radius: 10px; font-size: 0.95rem; outline: none;
            transition: all 0.2s ease;
        }
        select:focus, input:focus { border-color: var(--accent); box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.25); }
        button { background: linear-gradient(135deg, #6366f1, #4f46e5); border: none; font-weight: 700; cursor: pointer; display: inline-flex; align-items: center; gap: 0.5rem; }
        button:hover { transform: translateY(-2px); box-shadow: 0 4px 15px rgba(99, 102, 241, 0.4); }

        .tabs { display: flex; gap: 1rem; margin-bottom: 1rem; }
        .tab-btn { background: transparent; border: 1px solid var(--border); color: var(--text-secondary); }
        .tab-btn.active { background: var(--accent); color: white; border-color: var(--accent); }

        main { flex: 1; max-width: 1300px; width: 100%; margin: 0 auto; padding: 2rem 1.5rem; display: flex; flex-direction: column; gap: 2rem; }
        
        .search-box { display: flex; gap: 1rem; width: 100%; max-width: 700px; margin: 0 auto; }
        .search-box input { flex: 1; font-size: 1.1rem; padding: 0.8rem 1.2rem; }

        .manga-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 1.75rem; }
        .manga-card {
            background: var(--bg-card); border-radius: 16px; border: 1px solid var(--border);
            display: flex; flex-direction: column; overflow: hidden;
            transition: all 0.3s ease; position: relative;
        }
        .manga-card:hover { transform: translateY(-6px); border-color: var(--accent); box-shadow: 0 16px 35px rgba(0, 0, 0, 0.6); }
        
        .cover-box { position: relative; width: 100%; height: 300px; background: #0f172a; overflow: hidden; }
        .cover-img { width: 100%; height: 100%; object-fit: cover; transition: transform 0.4s ease; }
        .manga-card:hover .cover-img { transform: scale(1.05); }

        .card-details { padding: 1.25rem; display: flex; flex-direction: column; gap: 0.75rem; flex: 1; justify-content: space-between; }
        .manga-title { font-size: 1.1rem; font-weight: 700; line-height: 1.4; color: var(--text); }
        .author-tag { font-size: 0.85rem; color: var(--accent-glow); font-weight: 600; }
        .manga-desc { font-size: 0.82rem; color: var(--text-secondary); line-height: 1.4; display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }

        .card-actions { display: flex; gap: 0.5rem; margin-top: 0.5rem; flex-wrap: wrap; }
        .card-actions button { flex: 1; padding: 0.5rem; font-size: 0.85rem; justify-content: center; }

        /* Modal Drawer */
        .modal-overlay {
            position: fixed; inset: 0; background: rgba(0, 0, 0, 0.8); backdrop-filter: blur(8px);
            display: none; justify-content: center; align-items: center; z-index: 200; padding: 1rem;
        }
        .modal-overlay.active { display: flex; }
        .modal-content {
            background: #1e293b; border: 1px solid var(--border); border-radius: 20px;
            width: 100%; max-width: 750px; max-height: 85vh; display: flex; flex-direction: column; overflow: hidden;
        }
        .modal-header { padding: 1.25rem 1.5rem; border-bottom: 1px solid var(--border); display: flex; justify-content: space-between; align-items: center; background: rgba(15, 23, 42, 0.6); }
        .modal-body { padding: 1.5rem; overflow-y: auto; display: flex; flex-direction: column; gap: 0.75rem; }
        .close-btn { background: transparent; border: none; font-size: 1.5rem; color: var(--text-secondary); cursor: pointer; }
        
        .chapter-row {
            background: rgba(15, 23, 42, 0.6); padding: 0.75rem 1rem; border-radius: 10px;
            display: flex; justify-content: space-between; align-items: center; transition: background 0.2s;
        }
        .chapter-row:hover { background: rgba(99, 102, 241, 0.15); }
        .ch-title { font-weight: 600; font-size: 0.95rem; }

        /* Reader Overlay */
        .reader-view {
            position: fixed; inset: 0; background: #070a12; z-index: 300;
            display: none; flex-direction: column; overflow-y: auto; align-items: center;
        }
        .reader-view.active { display: flex; }
        .reader-nav {
            position: sticky; top: 0; width: 100%; background: rgba(7, 10, 18, 0.9); backdrop-filter: blur(12px);
            border-bottom: 1px solid var(--border); padding: 1rem 2rem; display: flex; justify-content: space-between; align-items: center; z-index: 310;
        }
        .reader-body { width: 100%; max-width: 900px; padding: 2rem 1rem; display: flex; flex-direction: column; gap: 1rem; align-items: center; }
        .reader-img { width: 100%; border-radius: 8px; min-height: 250px; background: #1e293b; object-fit: contain; }

        .spinner { border: 3px solid rgba(255,255,255,0.1); border-top: 3px solid var(--accent); border-radius: 50%; width: 24px; height: 24px; animation: spin 1s linear infinite; }
        @keyframes spin { 0% { transform: rotate(0deg); } 100% { transform: rotate(360deg); } }
    </style>
</head>
<body>
    <header>
        <div class="brand">🔥 Manga Source Web UI</div>
        <div class="controls-row">
            <div class="tabs">
                <button class="tab-btn active" id="tab-search" onclick="switchTab('search')">🔍 البحث والمصادر</button>
                <button class="tab-btn" id="tab-library" onclick="switchTab('library')">📚 المكتبة الشخصية</button>
                <button class="tab-btn" id="tab-offline" onclick="switchTab('offline')">📁 المكونات المنزلة (أوفلاين)</button>
            </div>
            <label style="display: flex; align-items: center; gap: 0.4rem; font-size: 0.88rem; cursor: pointer; color: #10b981;">
                <input type="checkbox" id="compress-toggle" style="width: auto; cursor: pointer;"> 📦 ضغط WebP (توفير 50% المساحة)
            </label>
            <label style="display: flex; align-items: center; gap: 0.4rem; font-size: 0.88rem; cursor: pointer; color: var(--accent-glow);">
                <input type="checkbox" id="aria2-toggle" style="width: auto; cursor: pointer;"> 🚀 تسريع التنزيل عبر aria2c
            </label>
            <select id="source-select">
                <!-- Dynamic JSON Sources -->
            </select>
        </div>
    </header>

    <main>
        <div id="view-search">
            <div class="search-box">
                <input type="text" id="search-input" placeholder="ابحث عن المانجا هنا (مثال: One Piece, Naruto...)" value="One Piece">
                <button id="search-btn">🔍 بحث</button>
            </div>

            <div id="results-container" class="manga-grid" style="margin-top: 2rem;">
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

    <!-- Chapters Modal -->
    <div class="modal-overlay" id="chapters-modal">
        <div class="modal-content">
            <div class="modal-header">
                <h3 id="modal-manga-title">فصول المانجا والتفاصيل</h3>
                <button class="close-btn" onclick="closeModal()">&times;</button>
            </div>
            <div class="modal-body" id="modal-chapters-list">
                <!-- Chapters list -->
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
        const searchInput = document.getElementById('search-input');
        const searchBtn = document.getElementById('search-btn');
        const aria2Toggle = document.getElementById('aria2-toggle');
        const compressToggle = document.getElementById('compress-toggle');
        const resultsContainer = document.getElementById('results-container');
        const libraryContainer = document.getElementById('library-container');
        const offlineContainer = document.getElementById('offline-container');
        const chaptersModal = document.getElementById('chapters-modal');
        const modalTitle = document.getElementById('modal-manga-title');
        const modalList = document.getElementById('modal-chapters-list');
        const readerView = document.getElementById('reader-view');
        const readerPages = document.getElementById('reader-pages');
        const readerTitle = document.getElementById('reader-title');

        let currentTab = 'search';

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
                const sources = await res.json();
                sourceSelect.innerHTML = '';
                sources.forEach(src => {
                    const opt = document.createElement('option');
                    opt.value = src.id;
                    opt.innerText = src.name;
                    if (src.id === '3asq') opt.selected = true;
                    sourceSelect.appendChild(opt);
                });
                performSearch();
            } catch (e) {
                console.error('Failed to load sources list', e);
            }
        }

        async function performSearch() {
            const query = searchInput.value.trim();
            const source = sourceSelect.value;
            if (!query) return;

            resultsContainer.innerHTML = '<div style="grid-column: 1/-1; text-align: center; padding: 3rem;"><div class="spinner" style="margin: 0 auto 1rem;"></div>جاري استخراج النتائج والأغلفة...</div>';

            try {
                const res = await fetch(`/api/search?source=${source}&q=${encodeURIComponent(query)}`);
                const data = await res.json();
                
                resultsContainer.innerHTML = '';
                if (!data || data.length === 0) {
                    resultsContainer.innerHTML = '<div style="grid-column: 1/-1; text-align: center; padding: 3rem; color: var(--text-secondary);">لم يتم العثور على نتائج.</div>';
                    return;
                }

                data.forEach(manga => {
                    const coverUrl = manga.cover_url ? `/api/proxy?url=${encodeURIComponent(manga.cover_url)}` : 'https://via.placeholder.com/300x400?text=No+Cover';
                    const card = document.createElement('div');
                    card.className = 'manga-card';
                    card.innerHTML = `
                        <div class="cover-box">
                            <img class="cover-img" src="${coverUrl}" alt="${manga.title}" loading="lazy">
                        </div>
                        <div class="card-details">
                            <div>
                                <h3 class="manga-title">${manga.title}</h3>
                                ${manga.author ? `<div class="author-tag">👤 ${manga.author}</div>` : ''}
                                ${manga.description ? `<p class="manga-desc">${manga.description}</p>` : ''}
                            </div>
                            <div class="card-actions">
                                <button onclick="fetchChapters('${manga.id}', '${manga.title.replace(/'/g, "\\'")}')">📚 الفصول</button>
                                <button style="background: #a855f7;" onclick="addToLibrary('${manga.id}', '${manga.title.replace(/'/g, "\\'")}', '${manga.cover_url || ''}')">📌 للمكتبة</button>
                            </div>
                        </div>
                    `;
                    resultsContainer.appendChild(card);
                });
            } catch (e) {
                resultsContainer.innerHTML = `<div style="grid-column: 1/-1; text-align: center; color: #f87171;">خطأ أثناء البحث: ${e.message}</div>`;
            }
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
                                <button onclick="fetchChaptersForSource('${item.manga_id}', '${item.title.replace(/'/g, "\\'")}', '${item.source_id}')">📚 الفصول</button>
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
            modalTitle.innerText = `فصول: ${mangaTitle}`;
            modalList.innerHTML = '<div style="text-align: center; padding: 2rem;"><div class="spinner" style="margin: 0 auto 1rem;"></div>جاري استخراج قائمة الفصول...</div>';
            chaptersModal.classList.add('active');

            try {
                const res = await fetch(`/api/chapters?source=${source}&id=${encodeURIComponent(mangaId)}`);
                const chapters = await res.json();

                modalList.innerHTML = '';
                if (!chapters || chapters.length === 0) {
                    modalList.innerHTML = '<div style="text-align: center; padding: 2rem; color: var(--text-secondary);">لا توجد فصول متاحة.</div>';
                    return;
                }

                chapters.forEach(ch => {
                    const row = document.createElement('div');
                    row.className = 'chapter-row';
                    row.innerHTML = `
                        <div class="ch-title">فصل ${ch.chapter_number} ${ch.title ? '- ' + ch.title : ''}</div>
                        <div style="display: flex; gap: 0.5rem;">
                            <button onclick="openReader('${ch.id}', 'فصل ${ch.chapter_number}', '${source}')">👁️ قراءة</button>
                            <button style="background: #10b981;" onclick="triggerDownload('${mangaId}', '${mangaTitle.replace(/'/g, "\\'")}', '${ch.chapter_number}', 'cbz', '${source}')">⬇️ CBZ</button>
                            <button style="background: #ef4444;" onclick="triggerDownload('${mangaId}', '${mangaTitle.replace(/'/g, "\\'")}', '${ch.chapter_number}', 'pdf', '${source}')">📄 PDF</button>
                        </div>
                    `;
                    modalList.appendChild(row);
                });
            } catch (e) {
                modalList.innerHTML = `<div style="color: #f87171; text-align: center; padding: 2rem;">خطأ: ${e.message}</div>`;
            }
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
        sourceSelect.addEventListener('change', performSearch);
        
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
