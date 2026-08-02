# ⚡ Manga Source (`manga-source`)

> Fast, lightweight, and modern Rust engine for searching, reading, tracking, and downloading manga across multiple sources.

![Rust](https://img.shields.io/badge/Rust-2021-orange?logo=rust)
![Axum](https://img.shields.io/badge/Axum-REST%20API-blue)
![Format](https://img.shields.io/badge/Formats-CBZ%20%7C%20PDF%20%7C%20WebP-green)

---

## ✨ Highlights & Features

- **🌐 Dynamic JSON Sources Engine**: Define custom sources with simple JSON files in `./custom_sources/`. Ships with 9 supported Arabic & English sources out of the box (MangaDex, 3asq, Mangalek, LikeManga, MangaLeko, MangaRead, MangaPill, FanFox, etc.).
- **🎨 Glassmorphic Web UI Dashboard & Reader**: Embedded HTTP Web App (`http://127.0.0.1:8080/`) for searching, reading chapters directly, and managing downloads.
- **📁 Offline Web Reader & Local Gallery**: Read downloaded CBZ files, PDFs, or image folders directly in your browser without internet connectivity.
- **📦 WebP Image Compression**: Convert page images to WebP on the fly (`--compress`), saving over **50% disk space** while preserving high quality.
- **🚀 High-Speed aria2c Acceleration**: Delegate image downloads to `aria2c` (`--use-aria2`) with 16 parallel connections for ultra-fast speeds.
- **📊 Download Queue & Pause/Resume**: Intelligent queue system (`queue.json`) allowing tasks to be paused, resumed, or retried.
- **📚 Personal Library Tracker & Auto-Updates**: Monitor tracked manga (`library.json`) and automatically discover and download new chapter releases (`manga-source update`).
- **🛡️ Cloudflare Session Manager & Cookies**: Store and automatically reuse `cf_clearance` cookies (`cookies.json`).
- **🏷️ ComicInfo.xml & PDF Metadata**: CBZ files are automatically packaged with standard `ComicInfo.xml` metadata for full Tachiyomi/Mihon compatibility.

---

## 🚀 Quick Start

### 1. Build & Run Web UI Server
```bash
cargo run --release -- server --port 8080
```
Open `http://127.0.0.1:8080/` in your browser!

### 2. Search & Download via CLI
```bash
# Search manga
cargo run --release -- --source 3asq search "One Piece"

# Download with aria2c & WebP compression as CBZ
cargo run --release -- --source 3asq download --id one-piece --chapters 1092 --format cbz --use-aria2 --compress
```

### 3. Personal Library Management
```bash
# Add manga to library tracker
cargo run --release -- library add --id one-piece --source 3asq --format cbz

# Auto-update library (checks for new chapters and downloads them)
cargo run --release -- update
```

---

## 🛠️ CLI Reference

| Subcommand | Description |
| :--- | :--- |
| `search <query>` | Search manga across selected source |
| `chapters --id <manga_id>` | List available chapters |
| `download --id <id> --chapters <range>` | Download chapters (CBZ, PDF, Folder) |
| `library <list\|add\|remove>` | Manage personal library tracker |
| `update` | Auto-download new chapter releases for tracked library |
| `queue <list\|pause\|resume\|clear>` | Download Queue manager |
| `cookies <list\|set\|clear>` | Cloudflare session & cookie manager |
| `server --port 8080` | Launch Axum REST API & Web Dashboard |
| `sources` | List all registered sources |

---

## 📜 License

MIT License. Open source and free for all manga lovers.
