use crate::models::{Chapter, GenreOption, Manga, MangaFilter, Page};
use crate::sources::MangaSource;
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;

pub struct MangaDexSource {
    client: Client,
    base_url: String,
}

impl MangaDexSource {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("manga-source-rust/0.1.0")
            .build()
            .unwrap_or_default();
        Self {
            client,
            base_url: "https://api.mangadex.org".to_string(),
        }
    }

    fn convert_manga_data(&self, item: MangaData) -> Manga {
        let title = item
            .attributes
            .title
            .get("en")
            .cloned()
            .or_else(|| item.attributes.title.get("ja-ro").cloned())
            .or_else(|| item.attributes.title.values().next().cloned())
            .unwrap_or_else(|| "Unknown Title".to_string());

        let description = item
            .attributes
            .description
            .and_then(|desc| desc.get("en").cloned().or_else(|| desc.get("ar").cloned()).or_else(|| desc.values().next().cloned()));

        let alt_titles: Vec<String> = item
            .attributes
            .alt_titles
            .into_iter()
            .filter_map(|map| map.into_values().next())
            .collect();

        let genres: Vec<String> = item
            .attributes
            .tags
            .into_iter()
            .filter_map(|t| t.attributes.name.get("en").cloned())
            .collect();

        let is_nsfw = item
            .attributes
            .content_rating
            .as_deref()
            .map(|r| r == "erotica" || r == "pornographic")
            .unwrap_or(false);

        let cover_file = item.relationships.iter().find_map(|r| {
            if r.rel_type == "cover_art" {
                r.attributes.as_ref().and_then(|a| a.file_name.clone())
            } else {
                None
            }
        });

        let author = item.relationships.iter().find_map(|r| {
            if r.rel_type == "author" {
                r.attributes.as_ref().and_then(|a| a.name.clone())
            } else {
                None
            }
        });

        let artist = item.relationships.iter().find_map(|r| {
            if r.rel_type == "artist" {
                r.attributes.as_ref().and_then(|a| a.name.clone())
            } else {
                None
            }
        });

        let cover_url = cover_file.map(|f| format!("https://uploads.mangadex.org/covers/{}/{}", item.id, f));

        Manga {
            id: item.id,
            title,
            alt_titles: if alt_titles.is_empty() { None } else { Some(alt_titles) },
            description,
            cover_url,
            author,
            artist,
            rating: None,
            rating_count: None,
            views: None,
            status: item.attributes.status,
            latest_chapter: item.attributes.latest_uploaded_chapter,
            updated_at: item.attributes.updated_at,
            genres: if genres.is_empty() { None } else { Some(genres) },
            tags: None,
            is_nsfw,
            manga_type: Some("Manga".to_string()),
            release_year: item.attributes.year.map(|y| y.to_string()),
            source_id: Some("mangadex".to_string()),
        }
    }
}

impl Default for MangaDexSource {
    fn default() -> Self {
        Self::new()
    }
}

// Serde DTOs for MangaDex API
#[derive(Deserialize)]
struct MangaListResponse {
    data: Vec<MangaData>,
}

#[derive(Deserialize)]
struct SingleMangaResponse {
    data: MangaData,
}

#[derive(Deserialize)]
struct MangaData {
    id: String,
    attributes: MangaAttributes,
    #[serde(default)]
    relationships: Vec<RelationshipData>,
}

#[derive(Deserialize)]
struct RelationshipData {
    #[allow(dead_code)]
    id: String,
    #[serde(rename = "type")]
    rel_type: String,
    attributes: Option<RelationshipAttributes>,
}

#[derive(Deserialize)]
struct RelationshipAttributes {
    #[serde(rename = "fileName")]
    file_name: Option<String>,
    name: Option<String>,
}

#[derive(Deserialize)]
struct TagData {
    attributes: TagAttributes,
}

#[derive(Deserialize)]
struct TagAttributes {
    name: HashMap<String, String>,
}

#[derive(Deserialize)]
struct MangaAttributes {
    title: HashMap<String, String>,
    #[serde(rename = "altTitles", default)]
    alt_titles: Vec<HashMap<String, String>>,
    description: Option<HashMap<String, String>>,
    status: Option<String>,
    #[serde(rename = "contentRating")]
    content_rating: Option<String>,
    #[serde(rename = "latestUploadedChapter")]
    latest_uploaded_chapter: Option<String>,
    #[serde(rename = "updatedAt")]
    updated_at: Option<String>,
    year: Option<u32>,
    #[serde(default)]
    tags: Vec<TagData>,
}

#[derive(Deserialize)]
struct ChapterFeedResponse {
    data: Vec<ChapterData>,
}

#[derive(Deserialize)]
struct ChapterData {
    id: String,
    attributes: ChapterAttributes,
}

#[derive(Deserialize)]
struct ChapterAttributes {
    chapter: Option<String>,
    title: Option<String>,
    #[serde(rename = "translatedLanguage")]
    translated_language: Option<String>,
    #[serde(rename = "publishAt")]
    publish_at: Option<String>,
}

#[derive(Deserialize)]
struct AtHomeResponse {
    #[serde(rename = "baseUrl")]
    base_url: String,
    chapter: AtHomeChapter,
}

#[derive(Deserialize)]
struct AtHomeChapter {
    hash: String,
    data: Vec<String>,
    #[serde(rename = "dataSaver", default)]
    data_saver: Vec<String>,
}

static MANGADEX_GENRES: &[(&str, &str, &str)] = &[
    ("action", "391b0423-d847-456f-aff0-8b0cfc03066b", "أكشن (Action)"),
    ("adventure", "87cc87cd-a395-47af-b27a-93258283bbc6", "مغامرة (Adventure)"),
    ("comedy", "4d32cc48-9f00-4cca-9b5a-a839f0764984", "كوميديا (Comedy)"),
    ("drama", "b9af3a63-f058-46de-a9a0-e0c13906197a", "دراما (Drama)"),
    ("fantasy", "cdc58593-87dd-415e-bbc0-2ec27bf404cc", "خيال (Fantasy)"),
    ("horror", "cdad7e68-1419-41dd-bdce-27753074a640", "رعب (Horror)"),
    ("isekai", "ace04997-f6bd-436e-b261-779182193d3d", "إيسيكاي (Isekai)"),
    ("mystery", "ee968100-4191-4968-93d3-f82d72be7e46", "غموض (Mystery)"),
    ("psychological", "3b60b75c-a2d7-4860-ab56-05f391bb889c", "نفسي (Psychological)"),
    ("romance", "423e2eae-a7a2-4a8b-ac03-a8351462d71d", "رومنسي (Romance)"),
    ("sci-fi", "256c8bd9-4904-4360-bf4f-508a76d67183", "خيال علمي (Sci-Fi)"),
    ("slice of life", "e5301a23-ebd9-49dd-a0cb-2add944c7fe9", "شريحة من الحياة (Slice of Life)"),
    ("sports", "69964a64-2f90-4d33-beeb-f3ed2875eb4c", "رياضة (Sports)"),
    ("supernatural", "eabc5b4c-6aff-42f3-b657-3e90cbd00b75", "قوى خارقة (Supernatural)"),
    ("thriller", "07251805-a27e-4d59-b488-f0bfbec15168", "إثارة وتشويق (Thriller)"),
    ("martial arts", "799c202e-7daa-44eb-9cf7-8a3c0441531e", "فنون قتالية (Martial Arts)"),
    ("historical", "33771934-028e-4cb3-8744-691e866a923e", "تاريخي (Historical)"),
    ("magic", "a1f53773-c69a-4ce5-8cab-fffcd90b1565", "سحر (Magic)"),
    ("monsters", "36fd93ea-e8b8-445e-b836-358f02b3d33d", "وحوش (Monsters)"),
    ("reincarnation", "0bc90acb-ccc1-44ca-a34a-b9f3a73259d0", "تناسخ الأرواح (Reincarnation)"),
    ("school life", "caaa44eb-cd40-4177-b930-79d3ef2afe87", "حياة مدرسية (School Life)"),
    ("survival", "5fff9cde-849c-4d78-aab0-0d52b2ee1d25", "نجاة (Survival)"),
    ("time travel", "292e862b-2d17-4062-90a2-0356caa4ae27", "سفر عبر الزمن (Time Travel)"),
    ("vampires", "d7d1730f-6eb0-4ba6-9437-602cac38664c", "مصاصو دماء (Vampires)"),
    ("villainess", "d14322ac-4d6f-4e9b-afd9-629d5f4d8a41", "الشريرة (Villainess)"),
    ("video games", "9438db5a-7e2a-4ac0-b39e-e0d95a34b8a8", "ألعاب فيديو (Video Games)"),
    ("mecha", "50880a9d-5440-4732-9afb-8f457127e836", "ميكا / آلات (Mecha)"),
    ("award winning", "0a39b5a1-b235-4886-a747-1d05d216532d", "حائز على جوائز (Award Winning)"),
    ("long strip", "3e2b8dae-350e-4ab8-a8ce-016e844b9f0d", "شريط طويل / ويبتون (Webtoon)"),
    ("full color", "f5ba408b-0e7a-484d-8d49-4e9125ac96de", "ملون بالكامل (Full Color)"),
];

#[async_trait]
impl MangaSource for MangaDexSource {
    fn id(&self) -> &str {
        "mangadex"
    }

    fn name(&self) -> &str {
        "MangaDex (Global Multi-Language API)"
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn languages(&self) -> Vec<String> {
        vec![
            "all".to_string(),
            "en".to_string(),
            "ar".to_string(),
            "ja".to_string(),
            "ko".to_string(),
            "zh".to_string(),
            "es".to_string(),
            "fr".to_string(),
            "ru".to_string(),
            "id".to_string(),
        ]
    }

    fn is_nsfw(&self) -> bool {
        false
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "api".to_string(),
            "global".to_string(),
            "multi-language".to_string(),
            "official-api".to_string(),
        ]
    }

    fn icon_url(&self) -> Option<String> {
        Some("https://mangadex.org/favicon.ico".to_string())
    }

    fn available_genres(&self) -> Vec<GenreOption> {
        MANGADEX_GENRES
            .iter()
            .map(|(id, _, name)| GenreOption {
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
        let url = format!("{}/manga", self.base_url);
        let limit_str = filter.limit.unwrap_or(24).to_string();
        let mut req = self.client.get(&url).query(&[
            ("limit", limit_str.as_str()),
            ("includes[]", "cover_art"),
            ("includes[]", "author"),
            ("includes[]", "artist"),
        ]);

        if filter.is_nsfw == Some(true) {
            req = req.query(&[
                ("contentRating[]", "safe"),
                ("contentRating[]", "suggestive"),
                ("contentRating[]", "erotica"),
                ("contentRating[]", "pornographic"),
            ]);
        } else {
            req = req.query(&[
                ("contentRating[]", "safe"),
                ("contentRating[]", "suggestive"),
            ]);
        }

        if let Some(q) = &filter.query {
            if !q.trim().is_empty() {
                req = req.query(&[("title", q.trim())]);
            }
        }

        if let Some(genre) = &filter.genre {
            let g_lower = genre.trim().to_lowercase();
            if let Some((_, tag_id, _)) = MANGADEX_GENRES.iter().find(|(k, _, label)| *k == g_lower || label.to_lowercase().contains(&g_lower)) {
                req = req.query(&[("includedTags[]", *tag_id)]);
            }
        }

        if let Some(st) = &filter.status {
            let s_lower = st.trim().to_lowercase();
            if s_lower == "ongoing" || s_lower == "مستمرة" {
                req = req.query(&[("status[]", "ongoing")]);
            } else if s_lower == "completed" || s_lower == "مكتملة" {
                req = req.query(&[("status[]", "completed")]);
            } else if s_lower == "hiatus" || s_lower == "متوقفة" {
                req = req.query(&[("status[]", "hiatus")]);
            } else if s_lower == "cancelled" || s_lower == "ملغاة" {
                req = req.query(&[("status[]", "cancelled")]);
            }
        }

        let order_by = filter.order_by.as_deref().unwrap_or("latest");
        match order_by {
            "rating" => {
                req = req.query(&[("order[rating]", "desc")]);
            }
            "views" | "popular" | "followed" => {
                req = req.query(&[("order[followedCount]", "desc")]);
            }
            "alphabet" => {
                req = req.query(&[("order[title]", "asc")]);
            }
            "newest" => {
                req = req.query(&[("order[createdAt]", "desc")]);
            }
            "year" => {
                req = req.query(&[("order[year]", "desc")]);
            }
            _ => {
                req = req.query(&[("order[latestUploadedChapter]", "desc")]);
            }
        }

        if let Some(mt) = &filter.manga_type {
            let mt_lower = mt.trim().to_lowercase();
            if mt_lower == "manga" || mt_lower == "مانجا" {
                req = req.query(&[("originalLanguage[]", "ja")]);
            } else if mt_lower == "manhwa" || mt_lower == "مانهوا" {
                req = req.query(&[("originalLanguage[]", "ko")]);
            } else if mt_lower == "manhua" || mt_lower == "مانها" {
                req = req.query(&[("originalLanguage[]", "zh")]);
            }
        }

        if let Some(demo) = &filter.demographic {
            let d_lower = demo.trim().to_lowercase();
            if d_lower == "shounen" || d_lower == "shoujo" || d_lower == "seinen" || d_lower == "josei" {
                req = req.query(&[("publicationDemographic[]", d_lower.as_str())]);
            }
        }

        let resp = req.send().await?.json::<MangaListResponse>().await?;
        let mangas = resp.data.into_iter().map(|item| self.convert_manga_data(item)).collect();
        Ok(mangas)
    }

    async fn get_manga_details(&self, manga_id: &str) -> Result<Option<Manga>> {
        let url = format!("{}/manga/{}", self.base_url, manga_id);
        let resp = self
            .client
            .get(&url)
            .query(&[
                ("includes[]", "cover_art"),
                ("includes[]", "author"),
                ("includes[]", "artist"),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(None);
        }

        let body: SingleMangaResponse = resp.json().await?;
        let manga = self.convert_manga_data(body.data);
        Ok(Some(manga))
    }

    async fn get_chapters(&self, manga_id: &str, lang: Option<&str>) -> Result<Vec<Chapter>> {
        let url = format!("{}/manga/{}/feed", self.base_url, manga_id);
        let target_lang = lang.unwrap_or("en");

        let resp = self
            .client
            .get(&url)
            .query(&[
                ("translatedLanguage[]", target_lang),
                ("order[chapter]", "asc"),
                ("limit", "500"),
            ])
            .send()
            .await?
            .json::<ChapterFeedResponse>()
            .await?;

        let chapters = resp
            .data
            .into_iter()
            .map(|item| {
                let ch_num = item.attributes.chapter.unwrap_or_else(|| "0".to_string());
                Chapter {
                    id: item.id,
                    chapter_number: ch_num,
                    title: item.attributes.title,
                    language: item.attributes.translated_language,
                    scanlator: None,
                    release_date: item.attributes.publish_at,
                    views: None,
                }
            })
            .collect();

        Ok(chapters)
    }

    async fn get_pages(&self, chapter_id: &str) -> Result<Vec<Page>> {
        let url = format!("{}/at-home/server/{}", self.base_url, chapter_id);
        let resp = self
            .client
            .get(&url)
            .send()
            .await?
            .json::<AtHomeResponse>()
            .await?;

        let (files, quality) = if !resp.chapter.data.is_empty() {
            (resp.chapter.data, "data")
        } else {
            (resp.chapter.data_saver, "data-saver")
        };

        let base_image_url = format!("{}/{}/{}", resp.base_url, quality, resp.chapter.hash);
        let pages = files
            .into_iter()
            .enumerate()
            .map(|(idx, filename)| {
                let page_url = format!("{}/{}", base_image_url, filename);
                let ext = filename.split('.').last().unwrap_or("jpg");
                Page {
                    index: idx + 1,
                    url: page_url,
                    filename: format!("{:03}.{}", idx + 1, ext),
                }
            })
            .collect();

        Ok(pages)
    }
}
