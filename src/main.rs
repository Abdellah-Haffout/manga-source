mod browser;
mod cookies;
mod downloader;
mod exporter;
mod library;
mod models;
mod queue;
mod server;
mod sources;

use anyhow::Result;
use browser::BrowserSession;
use clap::{Parser, Subcommand, ValueEnum};
use cookies::{CookieSession, CookieStore};
use downloader::Downloader;
use indicatif::MultiProgress;
use library::{LibraryItem, LibraryStore};
use models::{DownloadOptions, OutputFormat};
use queue::QueueStore;
use sources::json_source::JsonSource;
use sources::mangadex::MangaDexSource;
use sources::MangaSource;
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "manga-source")]
#[command(about = "Kotatsu-dl clone in Rust for downloading & reading manga", long_about = None)]
struct Cli {
    /// Select manga source provider (mangadex, or any custom JSON source id like 3asq, mangalek, etc.)
    #[arg(short = 's', long, default_value = "mangadex", global = true)]
    source: String,

    /// Path to a custom JSON source file (e.g. "./custom_sources/my_site.json")
    #[arg(long, global = true)]
    custom_source: Option<PathBuf>,

    /// Pass raw cookie string for Cloudflare clearance (e.g. "cf_clearance=xxx")
    #[arg(long, global = true)]
    cookie: Option<String>,

    /// Custom User-Agent header for matching browser clearance
    #[arg(long, global = true)]
    user_agent: Option<String>,

    /// Delegate image downloads to high-speed aria2c accelerator
    #[arg(long, global = true)]
    use_aria2: bool,

    /// Compress & convert page images to lightweight WebP format (saves 50%+ disk space)
    #[arg(long, global = true)]
    compress: bool,

    /// Launch browser window for interactive Cloudflare / CAPTCHA bypass
    #[arg(long, global = true)]
    bypass: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum FormatArg {
    Folder,
    Cbz,
    Pdf,
}

impl From<FormatArg> for OutputFormat {
    fn from(arg: FormatArg) -> Self {
        match arg {
            FormatArg::Folder => OutputFormat::Folder,
            FormatArg::Cbz => OutputFormat::Cbz,
            FormatArg::Pdf => OutputFormat::Pdf,
        }
    }
}

impl std::fmt::Display for FormatArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatArg::Folder => write!(f, "folder"),
            FormatArg::Cbz => write!(f, "cbz"),
            FormatArg::Pdf => write!(f, "pdf"),
        }
    }
}

#[derive(Subcommand)]
enum LibraryCommands {
    /// List all tracked manga in your personal library
    List,
    /// Add a manga to your personal library tracker
    Add {
        /// Manga ID
        #[arg(short, long)]
        id: String,

        /// Source provider ID (e.g. 3asq, mangadex, etc.)
        #[arg(short, long)]
        source: String,

        /// Custom title for manga (optional)
        #[arg(short, long)]
        title: Option<String>,

        /// Preferred download format (cbz, pdf, or folder)
        #[arg(short, long, value_enum, default_value_t = FormatArg::Cbz)]
        format: FormatArg,

        /// Output directory
        #[arg(short, long, default_value = "./downloads")]
        output: PathBuf,
    },
    /// Remove a manga from your personal library
    Remove {
        /// Manga ID to remove
        #[arg(short, long)]
        id: String,

        /// Source provider ID (optional)
        #[arg(short, long)]
        source: Option<String>,
    },
}

#[derive(Subcommand)]
enum CookiesCommands {
    /// List all saved cookie sessions
    List,
    /// Set a cookie session for a specific domain
    Set {
        /// Target domain name (e.g. 3asq.online)
        #[arg(short, long)]
        domain: String,

        /// Raw Cookie string (e.g. "cf_clearance=xxx; session=yyy")
        #[arg(short, long)]
        cookie: String,

        /// Custom User-Agent matching clearance (optional)
        #[arg(short, long)]
        user_agent: Option<String>,
    },
    /// Clear saved cookies for a domain
    Clear {
        /// Target domain name to remove
        #[arg(short, long)]
        domain: String,
    },
}

#[derive(Subcommand)]
enum QueueCommands {
    /// List all download queue items and their status
    List,
    /// Pause a downloading or pending task
    Pause {
        /// Task ID to pause
        #[arg(short, long)]
        id: String,
    },
    /// Resume a paused or failed task
    Resume {
        /// Task ID to resume
        #[arg(short, long)]
        id: String,
    },
    /// Clear completed items from download queue
    Clear,
}

#[derive(Subcommand)]
enum Commands {
    /// Search for manga title across sources
    Search {
        /// Manga title to search for
        query: String,
    },
    /// List available chapters for a manga ID
    Chapters {
        /// Manga ID
        #[arg(short, long)]
        id: String,
        /// Language code (e.g. "en", "ar")
        #[arg(short, long, default_value = "en")]
        lang: String,
    },
    /// Read chapter directly (stream page URLs or open interactive Web Reader)
    Read {
        /// Chapter ID to read
        #[arg(short, long)]
        id: String,

        /// Open interactive Web Reader directly in default browser
        #[arg(short, long)]
        open: bool,

        /// Server port for web reader
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
    },
    /// Personal Library manager (list, add, remove)
    Library {
        #[command(subcommand)]
        command: LibraryCommands,
    },
    /// Cookies & Cloudflare session manager (list, set, clear)
    Cookies {
        #[command(subcommand)]
        command: CookiesCommands,
    },
    /// Download Queue manager (list, pause, resume, clear)
    Queue {
        #[command(subcommand)]
        command: QueueCommands,
    },
    /// Check for new chapter releases across your library and download them automatically
    Update,
    /// Download chapters for a manga
    Download {
        /// Manga ID
        #[arg(short, long)]
        id: String,

        /// Title of manga (used for folder naming, optional)
        #[arg(short, long)]
        title: Option<String>,

        /// Chapter range or specific chapter (e.g., "1", "1-10", "all")
        #[arg(short, long, default_value = "all")]
        chapters: String,

        /// Output directory
        #[arg(short, long, default_value = "./downloads")]
        output: PathBuf,

        /// Output format (folder, cbz, or pdf)
        #[arg(short, long, value_enum, default_value_t = FormatArg::Folder)]
        format: FormatArg,

        /// Language code
        #[arg(short, long, default_value = "en")]
        lang: String,

        /// Number of concurrent page downloads
        #[arg(short = 'n', long, default_value_t = 4)]
        concurrent: usize,
    },
    /// Run local REST API server for GUI applications (Tauri, Flutter, Electron...)
    Server {
        /// Port to bind the HTTP REST server to
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
    },
    /// List available sources
    Sources,
}

fn get_source_from_cli(name: &str, custom_path: Option<&PathBuf>) -> Box<dyn MangaSource> {
    if let Some(path) = custom_path {
        if let Ok(js_src) = JsonSource::from_file(path) {
            return Box::new(js_src);
        }
    }

    let source_id = name.to_lowercase();
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.bypass {
        BrowserSession::launch_interactive_bypass("https://mangadex.org")?;
    }

    match cli.command {
        Commands::Server { port } => {
            server::start_server(port).await?;
        }
        Commands::Queue { command } => match command {
            QueueCommands::List => {
                let store = QueueStore::load();
                if store.items.is_empty() {
                    println!("Download queue is empty.");
                } else {
                    println!("Download Queue Manager ({} tasks):\n", store.items.len());
                    for (idx, q) in store.items.iter().enumerate() {
                        println!("{}. Task ID: {}", idx + 1, q.id);
                        println!("   Manga: {} (Ch. {})", q.manga_title, q.chapter_number);
                        println!(
                            "   Status: {:?} | Accelerator: {}",
                            q.status,
                            if q.use_aria2 { "aria2c" } else { "native" }
                        );
                        if let Some(err) = &q.error_message {
                            println!("   Error: {}", err);
                        }
                        println!();
                    }
                }
            }
            QueueCommands::Pause { id } => {
                let mut store = QueueStore::load();
                if store.pause(&id)? {
                    println!("Successfully paused download task '{}'.", id);
                } else {
                    println!("Task '{}' could not be paused or not found.", id);
                }
            }
            QueueCommands::Resume { id } => {
                let mut store = QueueStore::load();
                if store.resume(&id)? {
                    println!("Successfully resumed download task '{}'.", id);
                } else {
                    println!("Task '{}' could not be resumed or not found.", id);
                }
            }
            QueueCommands::Clear => {
                let mut store = QueueStore::load();
                store.clear_completed()?;
                println!("Cleared all completed tasks from download queue.");
            }
        },
        Commands::Cookies { command } => match command {
            CookiesCommands::List => {
                let store = CookieStore::load();
                if store.sessions.is_empty() {
                    println!("No saved cookie sessions.");
                } else {
                    println!("Saved Cloudflare & Domain Cookie Sessions ({} domains):\n", store.sessions.len());
                    for (idx, s) in store.sessions.iter().enumerate() {
                        println!("{}. Domain: {}", idx + 1, s.domain);
                        println!("   Cookie: {}", s.cookie_string);
                        if let Some(ua) = &s.user_agent {
                            println!("   User-Agent: {}", ua);
                        }
                        println!("   Updated: {}", s.updated_at);
                        println!();
                    }
                }
            }
            CookiesCommands::Set { domain, cookie, user_agent } => {
                let mut store = CookieStore::load();
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let session = CookieSession {
                    domain: domain.clone(),
                    cookie_string: cookie,
                    user_agent,
                    updated_at: format!("Timestamp {}", now),
                };
                store.set_session(session)?;
                println!("Successfully saved cookies for domain '{}'!", domain);
            }
            CookiesCommands::Clear { domain } => {
                let mut store = CookieStore::load();
                if store.clear_domain(&domain)? {
                    println!("Cleared cookies for domain '{}'.", domain);
                } else {
                    println!("No session found for domain '{}'.", domain);
                }
            }
        },
        Commands::Library { command } => match command {
            LibraryCommands::List => {
                let store = LibraryStore::load();
                if store.items.is_empty() {
                    println!("Library is currently empty.");
                } else {
                    println!("Personal Manga Library Tracker ({} items):\n", store.items.len());
                    for (idx, item) in store.items.iter().enumerate() {
                        println!(
                            "{}. {} (Source: {})",
                            idx + 1,
                            item.title,
                            item.source_id
                        );
                        println!("   ID: {}", item.manga_id);
                        println!(
                            "   Last Chapter: {} | Format: {}",
                            item.last_downloaded_chapter.as_deref().unwrap_or("None"),
                            item.preferred_format
                        );
                        println!();
                    }
                }
            }
            LibraryCommands::Add {
                id,
                source,
                title,
                format,
                output,
            } => {
                let mut store = LibraryStore::load();
                let manga_title = title.unwrap_or_else(|| format!("Manga_{}", id));
                let item = LibraryItem {
                    manga_id: id.clone(),
                    source_id: source.clone(),
                    title: manga_title.clone(),
                    cover_url: None,
                    last_downloaded_chapter: None,
                    preferred_format: format.to_string(),
                    output_dir: output.to_string_lossy().to_string(),
                };
                store.add(item)?;
                println!("Successfully added '{}' ({}) to your library tracker!", manga_title, source);
            }
            LibraryCommands::Remove { id, source } => {
                let mut store = LibraryStore::load();
                if store.remove(&id, source.as_deref())? {
                    println!("Removed manga ID '{}' from library.", id);
                } else {
                    println!("Manga ID '{}' not found in library.", id);
                }
            }
        },
        Commands::Update => {
            println!("Checking library tracker for new chapter releases...");
            let mut store = LibraryStore::load();
            if store.items.is_empty() {
                println!("Your library is empty. Add manga using 'manga-source library add' first!");
            } else {
                let updates = store
                    .check_and_update_all(|src_id| get_source_from_cli(src_id, None))
                    .await?;
                if updates.is_empty() {
                    println!("All tracked manga are up to date! No new chapters released.");
                } else {
                    println!("\nUpdate Summary ({} manga updated):\n", updates.len());
                    for u in updates {
                        println!("  - {}", u);
                    }
                }
            }
        }
        Commands::Read { id, open, port } => {
            let source = get_source_from_cli(&cli.source, cli.custom_source.as_ref());
            let reader_url = format!("http://127.0.0.1:{}/reader?source={}&id={}", port, cli.source, id);

            if open {
                println!("Launching Interactive Web Reader at {} ...", reader_url);
                let _ = open::that(&reader_url);
                server::start_server(port).await?;
            } else {
                println!("Fetching pages for Chapter ID '{}' on {}...", id, source.name());
                let pages = source.get_pages(&id).await?;
                if pages.is_empty() {
                    println!("No pages found for chapter '{}'", id);
                } else {
                    println!("\nFound {} pages:\n", pages.len());
                    for p in &pages {
                        println!("Page {:<3} | {}", p.index, p.url);
                    }
                    println!("\nTip: Run with '--open' to read directly in your browser!");
                }
            }
        }
        Commands::Sources => {
            let source = get_source_from_cli(&cli.source, cli.custom_source.as_ref());
            println!("Available Manga Sources (Selected: {}):", source.name());
            println!("  - mangadex   (MangaDex API v5 - Global Multi-Language API)");

            let custom_dir = PathBuf::from("./custom_sources");
            if custom_dir.exists() {
                if let Ok(entries) = fs::read_dir(custom_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map_or(false, |ext| ext == "json") {
                            if let Ok(js_src) = JsonSource::from_file(&path) {
                                println!("  - {:<10} ({} - Dynamic JSON)", js_src.id(), js_src.name());
                            }
                        }
                    }
                }
            }
        }
        Commands::Search { query } => {
            let source = get_source_from_cli(&cli.source, cli.custom_source.as_ref());
            println!("Searching for '{}' on {}...", query, source.name());
            let results = source.search(&query).await?;
            if results.is_empty() {
                println!("No manga found.");
            } else {
                println!("\nFound {} results:\n", results.len());
                for (idx, m) in results.iter().enumerate() {
                    println!("{}. {} (ID: {})", idx + 1, m.title, m.id);
                    if let Some(desc) = &m.description {
                        let short_desc: String = desc.chars().take(80).collect();
                        println!("   {}", short_desc.replace('\n', " "));
                    }
                    println!();
                }
            }
        }
        Commands::Chapters { id, lang } => {
            let source = get_source_from_cli(&cli.source, cli.custom_source.as_ref());
            println!("Fetching chapters for Manga ID '{}' on {} (Lang: {})...", id, source.name(), lang);
            let chapters = source.get_chapters(&id, Some(&lang)).await?;
            if chapters.is_empty() {
                println!("No chapters found.");
            } else {
                println!("\nFound {} chapters:\n", chapters.len());
                for ch in &chapters {
                    let title = ch.title.as_deref().unwrap_or("");
                    println!("Ch. {:<6} | ID: {} | {}", ch.chapter_number, ch.id, title);
                }
            }
        }
        Commands::Download {
            id,
            title,
            chapters: chapters_arg,
            output,
            format,
            lang,
            concurrent,
        } => {
            let source = get_source_from_cli(&cli.source, cli.custom_source.as_ref());
            let all_chapters = source.get_chapters(&id, Some(&lang)).await?;
            if all_chapters.is_empty() {
                println!("No chapters found for ID '{}'", id);
                return Ok(());
            }

            let manga_title = title.unwrap_or_else(|| format!("Manga_{}", id.replace('/', "_")));

            let target_chapters: Vec<_> = if chapters_arg.to_lowercase() == "all" {
                all_chapters
            } else if chapters_arg.contains('-') {
                let parts: Vec<&str> = chapters_arg.split('-').collect();
                if parts.len() == 2 {
                    let start: f32 = parts[0].parse().unwrap_or(0.0);
                    let end: f32 = parts[1].parse().unwrap_or(9999.0);
                    all_chapters
                        .into_iter()
                        .filter(|c| {
                            let num: f32 = c.chapter_number.parse().unwrap_or(-1.0);
                            num >= start && num <= end
                        })
                        .collect()
                } else {
                    all_chapters
                }
            } else {
                all_chapters
                    .into_iter()
                    .filter(|c| c.chapter_number == chapters_arg)
                    .collect()
            };

            println!(
                "Downloading {} chapters from {} for '{}' to {:?} ({:?}) [aria2c: {}, WebP Compress: {}]...",
                target_chapters.len(),
                source.name(),
                manga_title,
                output,
                format,
                cli.use_aria2,
                cli.compress
            );

            let options = DownloadOptions {
                output_dir: output,
                format: format.into(),
                concurrent_downloads: concurrent,
                language: Some(lang),
                cookies: cli.cookie,
                user_agent: cli.user_agent,
                use_aria2: cli.use_aria2,
                compress_webp: cli.compress,
            };

            let downloader = Downloader::new(options);
            let mp = MultiProgress::new();

            for ch in target_chapters {
                if let Err(e) = downloader.download_chapter(source.as_ref(), &manga_title, &ch, &mp).await {
                    eprintln!("Failed to download Ch. {}: {}", ch.chapter_number, e);
                }
            }
        }
    }

    Ok(())
}
