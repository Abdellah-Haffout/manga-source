use crate::cookies::CookieStore;
use crate::models::{Chapter, GenreOption, Manga, MangaFilter, Page};
use crate::sources::MangaSource;
use anyhow::Result;
use async_trait::async_trait;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegexExtractor {
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub id_group: usize,
    #[serde(default)]
    pub title_group: Option<usize>,
    #[serde(default)]
    pub cover_group: Option<usize>,
    #[serde(default)]
    pub alt_title_group: Option<usize>,
    #[serde(default)]
    pub author_group: Option<usize>,
    #[serde(default)]
    pub artist_group: Option<usize>,
    #[serde(default)]
    pub description_group: Option<usize>,
    #[serde(default)]
    pub rating_group: Option<usize>,
    #[serde(default)]
    pub views_group: Option<usize>,
    #[serde(default)]
    pub status_group: Option<usize>,
    #[serde(default)]
    pub latest_chapter_group: Option<usize>,
    #[serde(default)]
    pub updated_at_group: Option<usize>,

    // Block-level extraction
    #[serde(default)]
    pub item_pattern: Option<String>,
    #[serde(default)]
    pub id_regex: Option<String>,
    #[serde(default)]
    pub title_regex: Option<String>,
    #[serde(default)]
    pub cover_regex: Option<String>,
    #[serde(default)]
    pub alt_title_regex: Option<String>,
    #[serde(default)]
    pub author_regex: Option<String>,
    #[serde(default)]
    pub artist_regex: Option<String>,
    #[serde(default)]
    pub description_regex: Option<String>,
    #[serde(default)]
    pub rating_regex: Option<String>,
    #[serde(default)]
    pub views_regex: Option<String>,
    #[serde(default)]
    pub status_regex: Option<String>,
    #[serde(default)]
    pub latest_chapter_regex: Option<String>,
    #[serde(default)]
    pub updated_at_regex: Option<String>,
    #[serde(default)]
    pub genres_regex: Option<String>,
    #[serde(default)]
    pub tags_regex: Option<String>,
    #[serde(default)]
    pub nsfw_regex: Option<String>,
    #[serde(default)]
    pub type_regex: Option<String>,
    #[serde(default)]
    pub year_regex: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestStep {
    pub url_template: String,
    #[serde(default)]
    pub method: Option<String>,
    pub regex: RegexExtractor,
}

fn default_languages() -> Vec<String> {
    vec!["en".to_string()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonSourceConfig {
    pub id: String,
    pub name: String,
    pub base_url: String,
    #[serde(default = "default_languages")]
    pub languages: Vec<String>,
    #[serde(default)]
    pub is_nsfw: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub engine: Option<String>,
    pub search: RequestStep,
    pub latest: Option<RequestStep>,
    #[serde(default)]
    pub details: Option<RequestStep>,
    pub chapters: RequestStep,
    pub pages: RequestStep,
}

pub struct JsonSource {
    config: JsonSourceConfig,
    client: Client,
}

impl JsonSource {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: JsonSourceConfig = serde_json::from_str(&content)?;
        Self::from_config(config)
    }

    pub fn from_config(config: JsonSourceConfig) -> Result<Self> {
        let store = CookieStore::load();
        let session = store.get_session_for_domain(&config.base_url);

        let ua = session
            .and_then(|s| s.user_agent.as_deref())
            .unwrap_or_else(|| {
                config
                    .user_agent
                    .as_deref()
                    .unwrap_or("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
            });

        let mut headers = reqwest::header::HeaderMap::new();
        store.apply_headers_for_url(&config.base_url, &mut headers);
        if !headers.contains_key(reqwest::header::ACCEPT) {
            headers.insert(reqwest::header::ACCEPT, reqwest::header::HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8"));
        }
        if !headers.contains_key(reqwest::header::ACCEPT_LANGUAGE) {
            headers.insert(reqwest::header::ACCEPT_LANGUAGE, reqwest::header::HeaderValue::from_static("en-US,en;q=0.9,ar;q=0.8"));
        }

        let client = Client::builder()
            .user_agent(ua)
            .default_headers(headers)
            .danger_accept_invalid_certs(true)
            .timeout(std::time::Duration::from_secs(20))
            .build()?;

        Ok(Self { config, client })
    }

    fn normalize_url(&self, raw: &str) -> String {
        let trimmed = raw.trim();
        if trimmed.starts_with("//") {
            format!("https:{}", trimmed)
        } else if trimmed.starts_with('/') {
            let base = self.config.base_url.trim_end_matches('/');
            format!("{}{}", base, trimmed)
        } else {
            trimmed.to_string()
        }
    }

    fn extract_manga_from_html(&self, html: &str, step: &RequestStep) -> Result<Vec<Manga>> {
        let mut results = Vec::new();
        let mut seen = HashSet::new();

        if let Some(item_pattern) = &step.regex.item_pattern {
            let blocks: Vec<&str> = if let Some(delimiter) = item_pattern.strip_prefix("split:") {
                html.split(delimiter).skip(1).collect()
            } else {
                let item_re = Regex::new(item_pattern)?;
                item_re.find_iter(html).map(|m| m.as_str()).collect()
            };

            for block in blocks {
                if block.trim().is_empty() {
                    continue;
                }

                // ID
                let id = if let Some(id_p) = &step.regex.id_regex {
                    Regex::new(id_p)?
                        .captures(block)
                        .and_then(|c| c.get(1).or_else(|| c.get(0)))
                        .map(|m| m.as_str().trim().to_string())
                } else {
                    Regex::new(r#"href="https?://[^/]+/manga/([^/]+)/""#)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                };

                let id = match id {
                    Some(val) if !val.is_empty() => val,
                    _ => continue,
                };

                if !seen.insert(id.clone()) {
                    continue;
                }

                // Title
                let title = if let Some(t_p) = &step.regex.title_regex {
                    Regex::new(t_p)?
                        .captures(block)
                        .and_then(|c| c.get(1).or_else(|| c.get(0)))
                        .map(|m| m.as_str().trim().to_string())
                } else {
                    Regex::new(r#"<h[1-6][^>]*>\s*<a [^>]*>([^<]+)</a>"#)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                }.unwrap_or_else(|| id.clone());

                // Cover URL
                let cover_url = if let Some(c_p) = &step.regex.cover_regex {
                    Regex::new(c_p)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| self.normalize_url(m.as_str()))
                } else {
                    Regex::new(r#"<img [^>]*(?:src|data-src)="([^"]+)""#)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| self.normalize_url(m.as_str()))
                };

                // Alternative titles
                let alt_title = if let Some(a_p) = &step.regex.alt_title_regex {
                    Regex::new(a_p)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                } else {
                    Regex::new(r#"mg_alternative.*?<div class="summary-content">\s*([^<]+)"#)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                };
                let alt_titles = alt_title.map(|t| vec![t]);

                // Author
                let author = if let Some(au_p) = &step.regex.author_regex {
                    Regex::new(au_p)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                } else {
                    Regex::new(r#"mg_author.*?<div class="summary-content">\s*([^<]+)"#)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                };

                // Artist
                let artist = if let Some(ar_p) = &step.regex.artist_regex {
                    Regex::new(ar_p)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                } else {
                    None
                };

                // Description
                let description = if let Some(d_p) = &step.regex.description_regex {
                    Regex::new(d_p)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                } else {
                    None
                };

                // Rating
                let rating = if let Some(r_p) = &step.regex.rating_regex {
                    Regex::new(r_p)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .and_then(|m| m.as_str().trim().parse::<f32>().ok())
                } else {
                    Regex::new(r#"class="score font-meta total_votes">([0-9.]+)"#)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .and_then(|m| m.as_str().trim().parse::<f32>().ok())
                };

                // Views
                let views = if let Some(v_p) = &step.regex.views_regex {
                    Regex::new(v_p)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                } else {
                    Regex::new(r#"class="views">.*?</i>\s*([^<]+)</span>"#)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                };

                // Status
                let status = if let Some(s_p) = &step.regex.status_regex {
                    Regex::new(s_p)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                } else {
                    Regex::new(r#"mg_status.*?<div class="summary-content">\s*([^<]+)"#)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                };

                // Latest Chapter
                let latest_chapter = if let Some(lc_p) = &step.regex.latest_chapter_regex {
                    Regex::new(lc_p)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                } else {
                    Regex::new(r#"<span class="chapter font-meta">\s*<a[^>]*>\s*([^<]+)\s*</a>"#)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                        .or_else(|| {
                            Regex::new(r#"latest-chap.*?<span class="font-meta chapter"><a [^>]*>([^<]+)</a>"#).ok()?
                                .captures(block)?
                                .get(1)
                                .map(|m| m.as_str().trim().to_string())
                        })
                };

                // Updated at
                let updated_at = if let Some(u_p) = &step.regex.updated_at_regex {
                    Regex::new(u_p)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                } else {
                    Regex::new(r#"<span class="timediff">([^<]+)</span>"#)?
                        .captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                };

                // Genres
                let genres: Vec<String> = if let Some(g_p) = &step.regex.genres_regex {
                    Regex::new(g_p)?
                        .captures_iter(block)
                        .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
                        .collect()
                } else {
                    Regex::new(r#"href="https?://[^/]+/manga-genre/[^/]+/"[^>]*>([^<]+)</a>"#)?
                        .captures_iter(block)
                        .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
                        .collect()
                };

                // Badges / Tags
                let tags: Vec<String> = if let Some(tg_p) = &step.regex.tags_regex {
                    Regex::new(tg_p)?
                        .captures_iter(block)
                        .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
                        .collect()
                } else {
                    Regex::new(r#"<span class="manga-title-badges[^"]*">\s*(?:<span class="text">)?([^<]+)(?:</span>)?\s*</span>"#)?
                        .captures_iter(block)
                        .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
                        .collect()
                };

                // NSFW Check
                let is_nsfw = if let Some(nsfw_p) = &step.regex.nsfw_regex {
                    Regex::new(nsfw_p)?.is_match(block)
                } else {
                    self.config.is_nsfw
                        || tags.iter().any(|t| t.contains("18+") || t.to_lowercase().contains("adult") || t.to_lowercase().contains("hentai"))
                        || block.contains("18+")
                        || block.contains("adult")
                };

                results.push(Manga {
                    id,
                    title,
                    alt_titles,
                    description,
                    cover_url,
                    author,
                    artist,
                    rating,
                    rating_count: None,
                    views,
                    status,
                    latest_chapter,
                    updated_at,
                    genres: if genres.is_empty() { None } else { Some(genres) },
                    tags: if tags.is_empty() { None } else { Some(tags) },
                    is_nsfw,
                    manga_type: Some("Manga".to_string()),
                    release_year: None,
                    source_id: Some(self.config.id.clone()),
                });
            }
        } else if !step.regex.pattern.is_empty() {
            let re = Regex::new(&step.regex.pattern)?;
            for cap in re.captures_iter(html) {
                let id = cap
                    .get(step.regex.id_group)
                    .map(|m| m.as_str().trim().to_string())
                    .unwrap_or_default();

                if id.is_empty() || !seen.insert(id.clone()) {
                    continue;
                }

                let title = if let Some(grp) = step.regex.title_group {
                    cap.get(grp).map(|m| m.as_str().trim().to_string()).unwrap_or_else(|| id.clone())
                } else {
                    id.clone()
                };

                let cover_url = if let Some(grp) = step.regex.cover_group {
                    cap.get(grp).map(|m| self.normalize_url(m.as_str()))
                } else {
                    None
                };

                let alt_titles = step
                    .regex
                    .alt_title_group
                    .and_then(|g| cap.get(g))
                    .map(|m| vec![m.as_str().trim().to_string()]);

                let author = step
                    .regex
                    .author_group
                    .and_then(|g| cap.get(g))
                    .map(|m| m.as_str().trim().to_string());

                let artist = step
                    .regex
                    .artist_group
                    .and_then(|g| cap.get(g))
                    .map(|m| m.as_str().trim().to_string());

                let description = step
                    .regex
                    .description_group
                    .and_then(|g| cap.get(g))
                    .map(|m| m.as_str().trim().to_string());

                let rating = step
                    .regex
                    .rating_group
                    .and_then(|g| cap.get(g))
                    .and_then(|m| m.as_str().trim().parse::<f32>().ok());

                let views = step
                    .regex
                    .views_group
                    .and_then(|g| cap.get(g))
                    .map(|m| m.as_str().trim().to_string());

                let status = step
                    .regex
                    .status_group
                    .and_then(|g| cap.get(g))
                    .map(|m| m.as_str().trim().to_string());

                let latest_chapter = step
                    .regex
                    .latest_chapter_group
                    .and_then(|g| cap.get(g))
                    .map(|m| m.as_str().trim().to_string());

                let updated_at = step
                    .regex
                    .updated_at_group
                    .and_then(|g| cap.get(g))
                    .map(|m| m.as_str().trim().to_string());

                results.push(Manga {
                    id,
                    title,
                    alt_titles,
                    description,
                    cover_url,
                    author,
                    artist,
                    rating,
                    rating_count: None,
                    views,
                    status,
                    latest_chapter,
                    updated_at,
                    genres: None,
                    tags: None,
                    is_nsfw: self.config.is_nsfw,
                    manga_type: Some("Manga".to_string()),
                    release_year: None,
                    source_id: Some(self.config.id.clone()),
                });
            }
        }

        Ok(results)
    }

    fn extract_manga_details_from_html(&self, html: &str, manga_id: &str) -> Result<Manga> {
        let clean_html = Regex::new(r"(?s)<style.*?</style>")?.replace_all(html, "").to_string();

        let helper_field = |headings: &[&str]| -> Option<String> {
            for h in headings {
                let pattern = format!(r#"(?s)<h5>\s*{}\s*</h5>[\s\S]*?<div class="summary-content[^"]*">\s*([\s\S]*?)</div>"#, regex::escape(h));
                if let Ok(re) = Regex::new(&pattern) {
                    if let Some(c) = re.captures(&clean_html) {
                        if let Some(m) = c.get(1) {
                            let val = Regex::new(r"<[^>]+>").unwrap().replace_all(m.as_str(), "").trim().to_string();
                            if !val.is_empty() && val != "-" {
                                return Some(val);
                            }
                        }
                    }
                }
            }
            None
        };

        // Title
        let title = Regex::new(r#"(?s)<div class="post-title[^"]*">\s*(?:<span[^>]*>.*?</span>\s*)*<h1[^>]*>\s*([^<]+)"#)?
            .captures(&clean_html)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_else(|| manga_id.to_string());

        // Cover
        let cover_url = Regex::new(r#"(?s)<div class="summary_image"[^>]*>[\s\S]*?<img [^>]*(?:src|data-src)="([^"]+)""#)?
            .captures(&clean_html)
            .and_then(|c| c.get(1))
            .map(|m| self.normalize_url(m.as_str()));

        // Rating
        let rating = Regex::new(r#"class="score font-meta total_votes">([0-9.]+)"#)?
            .captures(&clean_html)
            .or_else(|| Regex::new(r#"id="averagerate">\s*([0-9.]+)"#).unwrap().captures(&clean_html))
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().trim().parse::<f32>().ok());

        // Rating Count
        let rating_count = Regex::new(r#"id="countrate">\s*([0-9]+)"#)?
            .captures(&clean_html)
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().trim().parse::<u32>().ok());

        // Alternative Titles
        let alt_title = helper_field(&["أسماء أخرى", "Alternative", "Alternative Titles", "Other Names"]);
        let alt_titles = alt_title.map(|t| vec![t]);

        // Author
        let author = helper_field(&["الكاتب", "المؤلف", "Author", "Authors"]);

        // Artist
        let artist = helper_field(&["الرسام", "Artist", "Artists"]);

        // Genres
        let genres_re = Regex::new(r#"href="https?://[^/]+/manga-genre/[^/]+/\"[^>]*>([^<]+)</a>"#)?;
        let genres: Vec<String> = genres_re
            .captures_iter(&clean_html)
            .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
            .filter(|g| !g.is_empty())
            .collect();

        // Status
        let status = helper_field(&["الحالة", "Status"]);

        // Type
        let manga_type = helper_field(&["النوع", "Type"]);

        // Release Year
        let release_year = helper_field(&["سنة الإصدار", "Release", "Release Year"]);

        // Views
        let views = helper_field(&["الترتيب", "المشاهدات", "Rank", "Views"])
            .or_else(|| {
                Regex::new(r#"<span class="views"><i class="fa fa-eye"></i>\s*([^<]+)</span>"#).ok()
                    .and_then(|re| re.captures(&clean_html))
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().trim().to_string())
            });

        // Tags / Badges
        let badges_re = Regex::new(r#"<span class="manga-title-badges[^"]*">\s*(?:<span class="text">)?([^<]+)(?:</span>)?\s*</span>"#)?;
        let tags: Vec<String> = badges_re
            .captures_iter(&clean_html)
            .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
            .filter(|t| !t.is_empty())
            .collect();

        let is_nsfw = tags.iter().any(|t| {
            let lower = t.to_lowercase();
            lower.contains("18+") || lower.contains("adult") || lower.contains("mature")
        }) || self.config.is_nsfw;

        // Description
        let desc_re1 = Regex::new(r#"(?s)<div class="manga-excerpt[^"]*">\s*<p>\s*([\s\S]*?)</p>"#)?;
        let desc_re2 = Regex::new(r#"(?s)<div class="description-summary[^"]*">[\s\S]*?<div class="summary__content[^"]*">\s*<p>\s*([\s\S]*?)</p>"#)?;
        let description = desc_re1
            .captures(&clean_html)
            .or_else(|| desc_re2.captures(&clean_html))
            .and_then(|c| c.get(1))
            .map(|m| {
                Regex::new(r"<[^>]+>").unwrap().replace_all(m.as_str(), "").trim().to_string()
            });

        Ok(Manga {
            id: manga_id.to_string(),
            title,
            alt_titles,
            description,
            cover_url,
            author,
            artist,
            rating,
            rating_count,
            views,
            status,
            latest_chapter: None,
            updated_at: None,
            genres: if genres.is_empty() { None } else { Some(genres) },
            tags: if tags.is_empty() { None } else { Some(tags) },
            is_nsfw,
            manga_type,
            release_year,
            source_id: Some(self.config.id.clone()),
        })
    }
}

static WP_MANGA_GENRES: &[(&str, &str)] = &[
    ("action", "أكشن (Action)"),
    ("adventure", "مغامرة (Adventure)"),
    ("comedy", "كوميديا (Comedy)"),
    ("drama", "دراما (Drama)"),
    ("fantasy", "خيال (Fantasy)"),
    ("horror", "رعب (Horror)"),
    ("isekai", "إيسيكاي (Isekai)"),
    ("mystery", "غموض (Mystery)"),
    ("romance", "رومنسي (Romance)"),
    ("sci-fi", "خيال علمي (Sci-Fi)"),
    ("shounen", "شونين (Shounen)"),
    ("seinen", "سينين (Seinen)"),
    ("shoujo", "شوجو (Shoujo)"),
    ("slice-of-life", "شريحة من الحياة (Slice of Life)"),
    ("sports", "رياضة (Sports)"),
    ("supernatural", "قوى خارقة (Supernatural)"),
    ("martial-arts", "فنون قتالية (Martial Arts)"),
    ("historical", "تاريخي (Historical)"),
    ("magic", "سحر (Magic)"),
    ("monsters", "وحوش (Monsters)"),
    ("school-life", "مدرسي (School Life)"),
    ("webtoon", "ويبتون (Webtoon)"),
    ("manhwa", "مانهوا (Manhwa)"),
    ("ecchi", "إيتشي (Ecchi)"),
    ("psychological", "نفسي (Psychological)"),
    ("tragedy", "مأساة (Tragedy)"),
    ("vampires", "مصاصو دماء (Vampires)"),
];

#[async_trait]
impl MangaSource for JsonSource {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn base_url(&self) -> &str {
        &self.config.base_url
    }

    fn languages(&self) -> Vec<String> {
        self.config.languages.clone()
    }

    fn is_nsfw(&self) -> bool {
        self.config.is_nsfw
    }

    fn tags(&self) -> Vec<String> {
        self.config.tags.clone()
    }

    fn icon_url(&self) -> Option<String> {
        self.config.icon_url.clone()
    }

    fn available_genres(&self) -> Vec<GenreOption> {
        WP_MANGA_GENRES
            .iter()
            .map(|(id, name)| GenreOption {
                id: id.to_string(),
                name: name.to_string(),
            })
            .collect()
    }

    async fn search(&self, query: &str) -> Result<Vec<Manga>> {
        self.filter_manga(&MangaFilter {
            query: if query.trim().is_empty() { None } else { Some(query.to_string()) },
            ..Default::default()
        }).await
    }

    async fn get_latest(&self) -> Result<Vec<Manga>> {
        self.filter_manga(&MangaFilter {
            order_by: Some("latest".to_string()),
            ..Default::default()
        }).await
    }

    async fn filter_manga(&self, filter: &MangaFilter) -> Result<Vec<Manga>> {
        let base = self.config.base_url.trim_end_matches('/');
        let is_wpmanga = self.config.tags.iter().any(|t| t.to_lowercase() == "wp-manga")
            || self.config.search.url_template.contains("wp-manga");

        let (mut url, step_to_use) = if let Some(q) = &filter.query {
            let u = self.config.search.url_template
                .replace("{base_url}", base)
                .replace("{query}", &q.replace(' ', "+"));
            (u, &self.config.search)
        } else if let Some(latest_step) = &self.config.latest {
            let u = latest_step.url_template
                .replace("{base_url}", base)
                .replace("{query}", "");
            (u, latest_step)
        } else if is_wpmanga {
            let order_param = match filter.order_by.as_deref().unwrap_or("latest") {
                "rating" => "rating",
                "views" | "popular" => "views",
                "alphabet" => "alphabet",
                "newest" => "new-manga",
                _ => "latest",
            };
            let u = format!("{}/?s=&post_type=wp-manga&m_orderby={}", base, order_param);
            (u, &self.config.search)
        } else {
            let u = self.config.search.url_template
                .replace("{base_url}", base)
                .replace("{query}", "");
            (u, &self.config.search)
        };

        if is_wpmanga {
            if let Some(genre) = &filter.genre {
                let g_lower = genre.trim().to_lowercase();
                if !g_lower.is_empty() && g_lower != "all" {
                    let slug = WP_MANGA_GENRES.iter().find(|(k, label)| *k == g_lower || label.to_lowercase().contains(&g_lower))
                        .map(|(k, _)| *k)
                        .unwrap_or(g_lower.as_str());
                    url.push_str(&format!("&genre%5B%5D={}", slug));
                }
            }

            if let Some(st) = &filter.status {
                let s_lower = st.trim().to_lowercase();
                if s_lower == "ongoing" || s_lower == "مستمرة" {
                    url.push_str("&status%5B%5D=on-going");
                } else if s_lower == "completed" || s_lower == "مكتملة" {
                    url.push_str("&status%5B%5D=completed");
                }
            }
        }

        let req = self.client.get(&url);
        let html = req.send().await?.text().await?;
        let mut mangas = self.extract_manga_from_html(&html, step_to_use)?;

        if mangas.is_empty() {
            if std::ptr::eq(step_to_use, &self.config.search) {
                if let Some(latest_step) = &self.config.latest {
                    mangas = self.extract_manga_from_html(&html, latest_step)?;
                }
            } else {
                mangas = self.extract_manga_from_html(&html, &self.config.search)?;
            }
        }

        if filter.is_nsfw == Some(false) {
            mangas.retain(|m| !m.is_nsfw);
        }

        Ok(mangas)
    }

    async fn get_manga_details(&self, manga_id: &str) -> Result<Option<Manga>> {
        let (url, method) = if let Some(step) = &self.config.details {
            let u = step.url_template
                .replace("{base_url}", &self.config.base_url)
                .replace("{manga_id}", manga_id);
            (u, step.method.as_deref().unwrap_or("GET"))
        } else {
            let base = self.config.base_url.trim_end_matches('/');
            (format!("{}/manga/{}/", base, manga_id), "GET")
        };

        let req = match method.to_uppercase().as_str() {
            "POST" => self.client.post(&url),
            _ => self.client.get(&url),
        };

        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Ok(None);
        }

        let html = resp.text().await?;
        let manga = self.extract_manga_details_from_html(&html, manga_id)?;
        Ok(Some(manga))
    }

    async fn get_chapters(&self, manga_id: &str, _lang: Option<&str>) -> Result<Vec<Chapter>> {
        let url = self
            .config
            .chapters
            .url_template
            .replace("{base_url}", &self.config.base_url)
            .replace("{manga_id}", manga_id);

        let req = match self.config.chapters.method.as_deref().unwrap_or("GET").to_uppercase().as_str() {
            "POST" => self.client.post(&url),
            _ => self.client.get(&url),
        };

        let html = req.send().await?.text().await?;
        let pattern = self
            .config
            .chapters
            .regex
            .pattern
            .replace("{manga_id}", &regex::escape(manga_id));

        let re = Regex::new(&pattern)?;
        let mut chapters = Vec::new();
        let mut seen = HashSet::new();

        for cap in re.captures_iter(&html) {
            let ch_id = cap
                .get(self.config.chapters.regex.id_group)
                .map(|m| m.as_str().trim())
                .unwrap_or("");

            let title_str = if let Some(grp) = self.config.chapters.regex.title_group {
                cap.get(grp).map(|m| m.as_str().trim().to_string())
            } else {
                None
            };

            let clean_num = ch_id
                .split('-')
                .last()
                .unwrap_or(ch_id)
                .trim()
                .to_string();

            if !ch_id.is_empty() && seen.insert(ch_id.to_string()) {
                let full_id = if ch_id.starts_with(manga_id) || ch_id.contains('/') {
                    ch_id.to_string()
                } else {
                    format!("{}/{}", manga_id, ch_id)
                };

                let ch_num = clean_num;

                chapters.push(Chapter {
                    id: full_id,
                    chapter_number: ch_num,
                    title: title_str,
                    language: self.config.languages.first().cloned(),
                    scanlator: None,
                    release_date: None,
                    views: None,
                });
            }
        }

        chapters.reverse();
        Ok(chapters)
    }

    async fn get_pages(&self, chapter_id: &str) -> Result<Vec<Page>> {
        let url = self
            .config
            .pages
            .url_template
            .replace("{base_url}", &self.config.base_url)
            .replace("{chapter_id}", chapter_id);

        let req = match self.config.pages.method.as_deref().unwrap_or("GET").to_uppercase().as_str() {
            "POST" => self.client.post(&url),
            _ => self.client.get(&url),
        };

        let html = req.send().await?.text().await?;
        let re = Regex::new(&self.config.pages.regex.pattern)?;

        let mut pages = Vec::new();

        for (idx, cap) in re.captures_iter(&html).enumerate() {
            if let Some(src_match) = cap.get(self.config.pages.regex.id_group) {
                let raw_img = src_match.as_str().trim();
                let img_url = self.normalize_url(raw_img);
                let ext = img_url.split('.').last().unwrap_or("jpg").split('?').next().unwrap_or("jpg").to_string();
                pages.push(Page {
                    index: idx + 1,
                    filename: format!("{:03}.{}", idx + 1, ext),
                    url: img_url,
                });
            }
        }

        Ok(pages)
    }
}
