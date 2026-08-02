use crate::models::Chapter;
use anyhow::{anyhow, Result};
use printpdf::*;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

pub struct Exporter;

impl Exporter {
    pub fn sanitize_filename(name: &str) -> String {
        name.chars()
            .map(|c| match c {
                '/' | '\\' | '?' | '%' | '*' | ':' | '|' | '"' | '<' | '>' | '.' => '_',
                _ => c,
            })
            .collect()
    }

    pub fn compress_images_to_webp<P: AsRef<Path>>(chapter_dir: P) -> Result<()> {
        let entries: Vec<_> = fs::read_dir(chapter_dir.as_ref())?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .collect();

        for entry in entries {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            if ext == "jpg" || ext == "jpeg" || ext == "png" {
                if let Ok(img) = ::image::open(&path) {
                    let new_path = path.with_extension("webp");
                    if let Ok(mut out_file) = File::create(&new_path) {
                        if img.write_to(&mut out_file, ::image::ImageFormat::WebP).is_ok() {
                            let _ = fs::remove_file(&path);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn generate_comic_info_xml(manga_title: &str, chapter: &Chapter) -> String {
        let ch_title = chapter.title.as_deref().unwrap_or("");
        let lang = chapter.language.as_deref().unwrap_or("ar");

        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<ComicInfo xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">
  <Title>{}</Title>
  <Series>{}</Series>
  <Number>{}</Number>
  <Summary>Chapter {} of {}</Summary>
  <LanguageISO>{}</LanguageISO>
</ComicInfo>"#,
            if ch_title.is_empty() {
                format!("Chapter {}", chapter.chapter_number)
            } else {
                ch_title.to_string()
            },
            manga_title,
            chapter.chapter_number,
            chapter.chapter_number,
            manga_title,
            lang
        )
    }

    pub fn export_cbz<P: AsRef<Path>>(
        chapter_dir: P,
        output_cbz_path: P,
        manga_title: &str,
        chapter: &Chapter,
    ) -> Result<()> {
        let file = File::create(output_cbz_path)?;
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        // 1. Embed ComicInfo.xml metadata
        let comic_info_xml = Self::generate_comic_info_xml(manga_title, chapter);
        zip.start_file("ComicInfo.xml", options)?;
        zip.write_all(comic_info_xml.as_bytes())?;

        // 2. Embed image pages
        let mut entries: Vec<_> = fs::read_dir(chapter_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .collect();

        entries.sort_by_key(|e| e.path());

        for entry in entries {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == "ComicInfo.xml" {
                    continue;
                }
                zip.start_file(name, options)?;
                let mut img_file = File::open(&path)?;
                let mut buffer = Vec::new();
                img_file.read_to_end(&mut buffer)?;
                zip.write_all(&buffer)?;
            }
        }

        zip.finish()?;
        Ok(())
    }

    pub fn export_pdf<P: AsRef<Path>>(
        chapter_dir: P,
        output_pdf_path: P,
        manga_title: &str,
    ) -> Result<()> {
        let mut entries: Vec<_> = fs::read_dir(chapter_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .collect();

        entries.sort_by_key(|e| e.path());

        if entries.is_empty() {
            return Err(anyhow!("No pages found to export to PDF"));
        }

        let first_img_path = entries[0].path();
        let (first_w, first_h) = ::image::image_dimensions(&first_img_path)?;
        let page_width = Mm(first_w as f32 * 0.264583);
        let page_height = Mm(first_h as f32 * 0.264583);

        let (doc, first_page_idx, first_layer_idx) = PdfDocument::new(manga_title, page_width, page_height, "Layer 1");

        for (idx, entry) in entries.iter().enumerate() {
            let img_path = entry.path();
            let (_current_page, current_layer) = if idx == 0 {
                (doc.get_page(first_page_idx), doc.get_page(first_page_idx).get_layer(first_layer_idx))
            } else {
                let (w, h) = ::image::image_dimensions(&img_path)?;
                let p_w = Mm(w as f32 * 0.264583);
                let p_h = Mm(h as f32 * 0.264583);
                let (p, l) = doc.add_page(p_w, p_h, "Layer 1");
                (doc.get_page(p), doc.get_page(p).get_layer(l))
            };

            let dynamic_img = ::image::open(&img_path)?;
            let (w, h) = (dynamic_img.width(), dynamic_img.height());
            let raw_bytes = dynamic_img.to_rgb8().into_raw();
            let image_xobject = ImageXObject {
                width: Px(w as usize),
                height: Px(h as usize),
                color_space: ColorSpace::Rgb,
                bits_per_component: ColorBits::Bit8,
                interpolate: true,
                image_data: raw_bytes,
                image_filter: None,
                smask: None,
                clipping_bbox: None,
            };

            let pdf_image = printpdf::Image::from(image_xobject);
            pdf_image.add_to_layer(current_layer, ImageTransform::default());
        }

        let pdf_file = File::create(output_pdf_path)?;
        let mut buf_writer = BufWriter::new(pdf_file);
        doc.save(&mut buf_writer)?;

        Ok(())
    }
}
