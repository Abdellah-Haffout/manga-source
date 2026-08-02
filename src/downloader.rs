use crate::cookies::CookieStore;
use crate::exporter::Exporter;
use crate::models::{Chapter, DownloadOptions, OutputFormat, Page};
use crate::sources::MangaSource;
use anyhow::Result;
use futures::stream::{self, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::process::Command as AsyncCommand;

pub struct Downloader {
    options: DownloadOptions,
    client: Client,
}

impl Downloader {
    pub fn new(options: DownloadOptions) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(cookie_str) = &options.cookies {
            if let Ok(val) = reqwest::header::HeaderValue::from_str(cookie_str) {
                headers.insert(reqwest::header::COOKIE, val);
            }
        }

        let default_ua = options
            .user_agent
            .as_deref()
            .unwrap_or("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36");

        let client = Client::builder()
            .user_agent(default_ua)
            .default_headers(headers)
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap_or_default();

        Self { options, client }
    }

    async fn download_with_aria2(
        &self,
        pages: &[Page],
        target_dir: &Path,
        pb: &ProgressBar,
    ) -> Result<bool> {
        let store = CookieStore::load();
        let input_path = target_dir.join("aria2_input.txt");
        let mut input_file = File::create(&input_path)?;

        let default_ua = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

        for page in pages {
            let session = store.get_session_for_domain(&page.url);
            let ua = session.and_then(|s| s.user_agent.as_deref()).unwrap_or(default_ua);

            writeln!(input_file, "{}", page.url)?;
            writeln!(input_file, "  out={}", page.filename)?;
            writeln!(input_file, "  dir={}", target_dir.to_string_lossy())?;
            writeln!(input_file, "  header=Referer: {}", page.url)?;
            writeln!(input_file, "  header=User-Agent: {}", ua)?;
            if let Some(s) = session {
                writeln!(input_file, "  header=Cookie: {}", s.cookie_string)?;
            }
            writeln!(input_file)?;
        }

        drop(input_file);

        pb.set_message("Delegating high-speed download to aria2c...");
        let status = AsyncCommand::new("aria2c")
            .arg("-i")
            .arg(&input_path)
            .arg("-j")
            .arg("8")
            .arg("-x")
            .arg("16")
            .arg("-s")
            .arg("16")
            .arg("--allow-overwrite=true")
            .arg("--auto-file-renaming=false")
            .arg("--console-log-level=warn")
            .status()
            .await;

        let _ = fs::remove_file(&input_path);

        match status {
            Ok(s) if s.success() => {
                pb.set_position(pages.len() as u64);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub async fn download_chapter(
        &self,
        source: &dyn MangaSource,
        manga_title: &str,
        chapter: &Chapter,
        mp: &MultiProgress,
    ) -> Result<PathBuf> {
        let pages = source.get_pages(&chapter.id).await?;
        if pages.is_empty() {
            return Err(anyhow::anyhow!("No pages found for chapter {}", chapter.chapter_number));
        }

        let safe_manga = Exporter::sanitize_filename(manga_title);
        let safe_chapter = Exporter::sanitize_filename(&chapter.chapter_number);
        let target_dir = self
            .options
            .output_dir
            .join(&safe_manga)
            .join(format!("Chapter_{}", safe_chapter));

        fs::create_dir_all(&target_dir)?;

        let pb = mp.add(ProgressBar::new(pages.len() as u64));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} pages (Ch. {msg})")
                .unwrap()
                .progress_chars("##-"),
        );
        pb.set_message(chapter.chapter_number.clone());

        let mut aria2_success = false;
        if self.options.use_aria2 {
            if let Ok(ok) = self.download_with_aria2(&pages, &target_dir, &pb).await {
                aria2_success = ok;
            }
        }

        if !aria2_success {
            let client = &self.client;
            let target_dir_ref = &target_dir;
            let pb_ref = &pb;
            let store = CookieStore::load();

            stream::iter(pages)
                .map(|page| {
                    let page_url = page.url.clone();
                    let file_path = target_dir_ref.join(&page.filename);
                    let store_ref = &store;
                    async move {
                        if !file_path.exists() {
                            let mut attempts = 0;
                            loop {
                                attempts += 1;
                                let mut req = client.get(&page_url).header("Referer", &page_url);
                                if let Some(session) = store_ref.get_session_for_domain(&page_url) {
                                    req = req.header("Cookie", &session.cookie_string);
                                    if let Some(ua) = &session.user_agent {
                                        req = req.header("User-Agent", ua);
                                    }
                                }

                                let res = req.send().await;
                                match res {
                                    Ok(response) if response.status().is_success() => {
                                        if let Ok(bytes) = response.bytes().await {
                                            if let Ok(mut f) = File::create(&file_path) {
                                                let _ = f.write_all(&bytes);
                                            }
                                        }
                                        break;
                                    }
                                    _ => {
                                        if attempts >= 3 {
                                            break;
                                        }
                                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                                    }
                                }
                            }
                        }
                        pb_ref.inc(1);
                    }
                })
                .buffer_unordered(self.options.concurrent_downloads)
                .collect::<Vec<()>>()
                .await;
        }

        if self.options.compress_webp {
            pb.set_message("Compressing & converting pages to WebP...");
            let _ = Exporter::compress_images_to_webp(&target_dir);
        }

        match self.options.format {
            OutputFormat::Folder => {
                pb.finish_with_message(format!("Downloaded Ch. {} to {:?}", chapter.chapter_number, target_dir));
            }
            OutputFormat::Cbz => {
                pb.set_message("Packing CBZ archive with ComicInfo.xml...");
                let cbz_name = format!("{}_Ch_{}.cbz", safe_manga, safe_chapter);
                let cbz_path = self.options.output_dir.join(cbz_name);
                if let Err(e) = Exporter::export_cbz(&target_dir, &cbz_path, manga_title, chapter) {
                    eprintln!("Failed to create CBZ archive: {}", e);
                } else {
                    let _ = fs::remove_dir_all(&target_dir);
                }
                pb.finish_with_message(format!("Created CBZ Ch. {}", chapter.chapter_number));
            }
            OutputFormat::Pdf => {
                pb.set_message("Exporting PDF document...");
                let pdf_name = format!("{}_Ch_{}.pdf", safe_manga, safe_chapter);
                let pdf_path = self.options.output_dir.join(pdf_name);
                if let Err(e) = Exporter::export_pdf(&target_dir, &pdf_path, manga_title) {
                    eprintln!("Failed to create PDF document: {}", e);
                } else {
                    let _ = fs::remove_dir_all(&target_dir);
                }
                pb.finish_with_message(format!("Created PDF Ch. {}", chapter.chapter_number));
            }
        }

        Ok(target_dir)
    }
}
