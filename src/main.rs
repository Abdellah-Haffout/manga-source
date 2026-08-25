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
use models::{DownloadOptions, MangaFilter, OutputFormat};
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
    #[arg(short, long, default_value = "3asq")]
    source: String,

    /// Path to a custom JSON source definition file
    #[arg(short, long)]
    custom_source: Option<PathBuf>,

    /// Pass raw cookie string for Cloudflare clearance
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
    /// Display latest updated manga from homepage without search query
    Latest,
    /// List all available genres, categories, and sort orders for the selected source
    Genres,
    /// Search for manga title across sources
    Search {
        /// Manga title to search for
        query: String,
    },
    /// Advanced Filter & Sort manga across genres, status, and order
    Filter {
        /// Search keyword / title
        #[arg(short, long)]
        query: Option<String>,

        /// Filter by genre / category (e.g. "action", "fantasy", "comedy")
        #[arg(short, long)]
        genre: Option<String>,

        /// Sort order ("latest", "rating", "views", "alphabet", "newest")
        #[arg(short, long, default_value = "latest")]
        order: String,

        /// Status filter ("ongoing", "completed", "hiatus", "all")
        #[arg(short, long)]
        status: Option<String>,

        /// Manga type filter ("manga", "manhwa", "manhua")
        #[arg(short = 't', long)]
        manga_type: Option<String>,

        /// Include 18+ Adult NSFW content
        #[arg(long)]
        nsfw: bool,
    },
    /// Display full details, synopsis, ratings, and stats for a manga
    Info {
        /// Manga ID
        #[arg(short, long)]
        id: String,
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

fn print_manga_details(idx: usize, m: &models::Manga) {
    println!("{}. {} (ID: {})", idx + 1, m.title, m.id);
    if let Some(alts) = &m.alt_titles {
        if !alts.is_empty() {
            println!("   Alternative: {}", alts.join(" | "));
        }
    }

    let mut meta_parts = Vec::new();
    if let Some(r) = m.rating {
        meta_parts.push(format!("⭐ Rating: {:.1}/5", r));
    }
    if let Some(v) = &m.views {
        meta_parts.push(format!("👁️ Views: {}", v));
    }
    if let Some(s) = &m.status {
        meta_parts.push(format!("🟢 Status: {}", s));
    }
    if let Some(ch) = &m.latest_chapter {
        meta_parts.push(format!("📌 Latest: Ch. {}", ch));
    }
    if let Some(up) = &m.updated_at {
        meta_parts.push(format!("🕒 Updated: {}", up));
    }
    if m.is_nsfw {
        meta_parts.push("🔞 18+ Adult".to_string());
    }

    if !meta_parts.is_empty() {
        println!("   {}", meta_parts.join(" | "));
    }

    if let Some(genres) = &m.genres {
        if !genres.is_empty() {
            println!("   Genres: {}", genres.join(", "));
        }
    }
    if let Some(tags) = &m.tags {
        if !tags.is_empty() {
            println!("   Tags: {}", tags.join(", "));
        }
    }

    if let Some(desc) = &m.description {
        let short_desc: String = desc.chars().take(120).collect();
        println!("   {}", short_desc.replace('\n', " "));
    }
    println!();
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
        Commands::Latest => {
            let source = get_source_from_cli(&cli.source, cli.custom_source.as_ref());
            println!("Fetching latest updated manga from {}...", source.name());
            let results = source.get_latest().await?;
            if results.is_empty() {
                println!("No latest manga found.");
            } else {
                println!("\nFound {} latest manga:\n", results.len());
                for (idx, m) in results.iter().enumerate() {
                    print_manga_details(idx, m);
                }
            }
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
            let m_src = MangaDexSource::new();
            println!(
                "  - {:<10} {:<35} [Lang: {} | Safe | Tags: {}]",
                m_src.id(),
                m_src.name(),
                m_src.languages().join(", "),
                m_src.tags().join(", ")
            );

            let custom_dir = PathBuf::from("./custom_sources");
            if custom_dir.exists() {
                if let Ok(entries) = fs::read_dir(custom_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map_or(false, |ext| ext == "json") {
                            if let Ok(js_src) = JsonSource::from_file(&path) {
                                let nsfw_tag = if js_src.is_nsfw() { "🔞 18+ Adult" } else { "Safe" };
                                println!(
                                    "  - {:<10} {:<35} [Lang: {} | {} | Tags: {}]",
                                    js_src.id(),
                                    js_src.name(),
                                    js_src.languages().join(", "),
                                    nsfw_tag,
                                    js_src.tags().join(", ")
                                );
                            }
                        }
                    }
                }
            }
        }
        Commands::Genres => {
            let source = get_source_from_cli(&cli.source, cli.custom_source.as_ref());
            let genres = source.available_genres();
            let sort_orders = source.available_sort_orders();
            println!("Available Genres & Categories for {}: ({} genres)\n", source.name(), genres.len());
            for (idx, g) in genres.iter().enumerate() {
                println!("  {:<3}. {:<25} (ID: {})", idx + 1, g.name, g.id);
            }
            println!("\nAvailable Sort Orders:");
            for (idx, s) in sort_orders.iter().enumerate() {
                println!("  {:<3}. {:<35} (ID: {})", idx + 1, s.name, s.id);
            }
            println!();
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
                    print_manga_details(idx, m);
                }
            }
        }
        Commands::Filter {
            query,
            genre,
            order,
            status,
            manga_type,
            nsfw,
        } => {
            let source = get_source_from_cli(&cli.source, cli.custom_source.as_ref());
            let filter = MangaFilter {
                query,
                genre,
                genres: None,
                status,
                order_by: Some(order),
                manga_type,
                demographic: None,
                language: None,
                is_nsfw: if nsfw { Some(true) } else { None },
                page: None,
                limit: Some(25),
            };

            println!("Filtering and sorting manga on {}...", source.name());
            let results = source.filter_manga(&filter).await?;
            if results.is_empty() {
                println!("No manga found matching the filter criteria.");
            } else {
                println!("\nFound {} results:\n", results.len());
                for (idx, m) in results.iter().enumerate() {
                    print_manga_details(idx, m);
                }
            }
        }
        Commands::Info { id } => {
            let source = get_source_from_cli(&cli.source, cli.custom_source.as_ref());
            println!("Fetching full details for Manga ID '{}' from {}...", id, source.name());
            if let Some(manga) = source.get_manga_details(&id).await? {
                println!("\n=======================================================");
                println!("📖 Title: {}", manga.title);
                if let Some(alt) = &manga.alt_titles {
                    println!("🔤 Alternative Titles: {}", alt.join(" | "));
                }
                if let Some(cover) = &manga.cover_url {
                    println!("🖼️ Cover Art: {}", cover);
                }
                if let Some(r) = manga.rating {
                    let count_str = manga.rating_count.map(|c| format!(" ({} votes)", c)).unwrap_or_default();
                    println!("⭐ Rating: {:.1}/5{}", r, count_str);
                }
                if let Some(v) = &manga.views {
                    println!("👁️ Views / Rank: {}", v);
                }
                if let Some(a) = &manga.author {
                    println!("👤 Author: {}", a);
                }
                if let Some(ar) = &manga.artist {
                    println!("🎨 Artist: {}", ar);
                }
                if let Some(s) = &manga.status {
                    println!("🟢 Status: {}", s);
                }
                if let Some(t) = &manga.manga_type {
                    println!("📚 Type: {}", t);
                }
                if let Some(y) = &manga.release_year {
                    println!("📅 Release Year: {}", y);
                }
                if manga.is_nsfw {
                    println!("🔞 Content: 18+ Adult / Mature");
                }
                if let Some(g) = &manga.genres {
                    println!("🏷️ Genres: {}", g.join(", "));
                }
                if let Some(t) = &manga.tags {
                    println!("🔖 Tags: {}", t.join(", "));
                }
                if let Some(d) = &manga.description {
                    println!("\n📝 Synopsis:\n{}", d);
                }
                println!("=======================================================\n");
            } else {
                println!("Could not fetch details for manga ID '{}'.", id);
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
