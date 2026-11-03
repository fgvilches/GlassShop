// ─────────────────────────────────────────────────────────────────────────────
// hShop catalog — port of src/lib/{hshop-api,parsers}.js to Rust.
// hshop.erista.me has no public JSON API: everything is scraped from the
// server-rendered HTML of site v3 (same selectors the JS client used).
// ─────────────────────────────────────────────────────────────────────────────

use once_cell::sync::Lazy;
use regex::Regex;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

pub const BASE: &str = "https://hshop.erista.me";
pub const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) hshop-glass/1.0 Chrome/138 Safari/537.36";

static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent(UA)
        .gzip(true)
        .build()
        .expect("http client")
});

pub fn client() -> &'static reqwest::Client {
    &CLIENT
}

// ── models ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TitleRow {
    pub id: String,
    pub url: String,
    pub name: String,
    pub category_path: String,
    pub content_type: Option<String>,
    pub title_id: Option<String>,
    pub product_code: Option<String>,
    pub version: Option<String>,
    pub size_text: Option<String>,
    pub size_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Subcategory {
    pub category: String,
    pub id: String,
    pub name: String,
    pub description: String,
    pub size_text: Option<String>,
    pub size_bytes: Option<i64>,
    pub titles: i64,
    pub official: i64,
    pub legit: Option<String>,
    pub downloads: i64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TitleDetail {
    pub id: String,
    pub url: String,
    pub name: String,
    pub size_text: Option<String>,
    pub size_bytes: Option<i64>,
    pub product_code: Option<String>,
    pub content_type: Option<String>,
    pub title_id: Option<String>,
    pub version: Option<String>,
    pub added: Option<String>,
    pub updated: Option<String>,
    pub downloads: Option<String>,
    pub seed: Option<String>,
    pub sha256: Option<String>,
    pub description: Option<String>,
    pub needs_captcha: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TitlePage {
    pub titles: Vec<TitleRow>,
    pub total: Option<i64>,
    pub offset: i64,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DownloadLinks {
    pub direct: Option<String>,
    pub qr: Option<String>,
    pub installer: Option<String>,
    pub qr_image: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TitlesQuery {
    pub category: Option<String>,
    pub subcategory: Option<String>,
    pub count: Option<i64>,
    pub offset: Option<i64>,
    pub sort_by: Option<String>,
    pub direction: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub query: Option<String>,
    pub qt: Option<String>,
    pub count: Option<i64>,
    pub offset: Option<i64>,
    /// include filters, `category.subcategory` joined by commas (site param `i`)
    pub include: Option<String>,
    /// exclude filters, same format (site param `e`)
    pub exclude: Option<String>,
    /// content legitimacy: standard | legit | piratelegit (site param `p`)
    pub content_type: Option<String>,
    /// sort field (site param `sb`)
    pub sort_by: Option<String>,
    /// ascending | descending (site param `sd`)
    pub direction: Option<String>,
}

pub fn categories() -> Vec<Category> {
    let raw = [
        ("games", "Games", "Nintendo 3DS games."),
        ("updates", "Updates", "Update data for Nintendo 3DS titles."),
        ("dlc", "DLC", "Downloadable content (DLC)."),
        ("virtual-console", "Virtual Console", "Official Virtual Console titles."),
        ("dsiware", "DSiWare", "Nintendo DSiWare for the 3DS."),
        ("videos", "Videos", "Videos, trailers or movies released on the 3DS."),
        ("extras", "Extras", "Miscellaneous content."),
        ("themes", "Themes", "Official 3DS themes in Anemone3DS format."),
    ];
    raw.iter()
        .map(|(id, name, description)| Category {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
        })
        .collect()
}

// ── http ─────────────────────────────────────────────────────────────────────

pub async fn fetch_text(url: &str) -> Result<String, String> {
    let res = client()
        .get(url)
        .header(reqwest::header::ACCEPT_LANGUAGE, "en")
        .send()
        .await
        .map_err(|e| format!("network error: {e}"))?;

    let status = res.status();
    if !status.is_success() {
        return Err(format!("{} on {}", status.as_u16(), url));
    }
    res.text().await.map_err(|e| format!("body error: {e}"))
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}

fn text_of(el: scraper::ElementRef) -> String {
    el.text().collect::<String>().trim().to_string()
}

static TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());
static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
static SHOWING_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)showing\s+(\d+)\s*[-\u{2013}]\s*(\d+)\s*of\s*(\d+)").unwrap());
static TITLE_HREF_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/t/(\d+)").unwrap());
static SUB_HREF_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/c/[^/]+/s/([^/?#]+)").unwrap());
static SIZE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)([\d.]+)\s*(TiB|GiB|MiB|KiB)").unwrap());
static META_LINE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^([A-Za-z0-9 .\-/]+?):\s*(.+)$").unwrap());

/// Strip tags to plain text, preserving word boundaries (needed for "showing
/// 1–25 of 92" which lives in loose markup).
fn plain_text(html: &str) -> String {
    let no_tags = TAG_RE.replace_all(html, " ");
    WS_RE.replace_all(&no_tags, " ").to_string()
}

pub fn size_to_bytes(text: &str) -> Option<i64> {
    let caps = SIZE_RE.captures(text)?;
    let value: f64 = caps.get(1)?.as_str().parse().ok()?;
    let mult: f64 = match caps.get(2)?.as_str().to_ascii_lowercase().as_str() {
        "tib" => 1024f64.powi(4),
        "gib" => 1024f64.powi(3),
        "mib" => 1024f64.powi(2),
        "kib" => 1024f64,
        _ => return None,
    };
    Some((value * mult).round() as i64)
}

// ── parsers ──────────────────────────────────────────────────────────────────

pub fn parse_title_rows(html: &str) -> Vec<TitleRow> {
    let doc = Html::parse_document(html);
    let entry_sel = sel("a.list-entry");
    let meta_sel = sel(".meta-content");
    let span_sel = sel("span");
    let h3_sel = sel(".base-info h3");
    let h4_sel = sel(".base-info h4");

    let mut out = Vec::new();
    for entry in doc.select(&entry_sel) {
        let href = entry.value().attr("href").unwrap_or("");
        let Some(caps) = TITLE_HREF_RE.captures(href) else { continue };
        let id = caps.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }

        // meta-content blocks are label/value pairs rendered as two <span>s
        let mut meta: Vec<(String, String)> = Vec::new();
        for block in entry.select(&meta_sel) {
            let spans: Vec<String> = block.select(&span_sel).map(text_of).collect();
            if spans.len() >= 2 {
                let value = spans.first().cloned().unwrap_or_default();
                let label = spans.last().cloned().unwrap_or_default();
                if !label.is_empty() {
                    meta.push((label, value));
                }
            }
        }
        let get = |key: &str| -> Option<String> {
            meta.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v.clone())
                .filter(|v| !v.is_empty())
        };

        let name = entry.select(&h3_sel).next().map(text_of).unwrap_or_default();
        let category_path = entry
            .select(&h4_sel)
            .next()
            .map(text_of)
            .unwrap_or_default()
            .trim_start_matches("content in ")
            .trim()
            .to_string();

        let size_text = get("Size");
        out.push(TitleRow {
            url: format!("{BASE}/t/{id}"),
            id,
            name,
            category_path,
            content_type: get("Content Type"),
            title_id: get("Title ID"),
            product_code: get("Product Code"),
            version: get("Version"),
            size_bytes: size_text.as_deref().and_then(size_to_bytes),
            size_text,
        });
    }
    out
}

pub fn parse_subcategory_rows(html: &str, category: &str) -> Vec<Subcategory> {
    let doc = Html::parse_document(html);
    let entry_sel = sel("a.list-entry");
    let meta_sel = sel(".meta-content");
    let span_sel = sel("span");
    let h3_sel = sel(".base-info h3");
    let h4_sel = sel(".base-info h4");

    let mut out = Vec::new();
    for entry in doc.select(&entry_sel) {
        let href = entry.value().attr("href").unwrap_or("");
        let Some(caps) = SUB_HREF_RE.captures(href) else { continue };
        let id = caps.get(1).map(|m| m.as_str()).unwrap_or("").to_string();

        let mut stats: Vec<(String, String)> = Vec::new();
        for block in entry.select(&meta_sel) {
            let spans: Vec<String> = block.select(&span_sel).map(text_of).collect();
            if spans.len() >= 2 {
                let label = spans.last().cloned().unwrap_or_default();
                let value = spans.first().cloned().unwrap_or_default();
                if !label.is_empty() {
                    stats.push((label, value));
                }
            }
        }
        let get = |key: &str| -> Option<String> {
            stats
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v.clone())
        };
        let num = |key: &str| -> i64 {
            get(key)
                .map(|v| v.replace(',', ""))
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0)
        };

        let size_text = get("Size");
        out.push(Subcategory {
            category: category.to_string(),
            id,
            name: entry.select(&h3_sel).next().map(text_of).unwrap_or_default(),
            description: entry.select(&h4_sel).next().map(text_of).unwrap_or_default(),
            size_bytes: size_text.as_deref().and_then(size_to_bytes),
            size_text,
            titles: num("Titles"),
            official: num("Official"),
            legit: get("Legit"),
            downloads: num("Downloads"),
        });
    }
    out
}

pub fn parse_showing(html: &str) -> Option<(i64, i64, i64)> {
    let text = plain_text(html);
    let caps = SHOWING_RE.captures(&text)?;
    Some((
        caps.get(1)?.as_str().parse().ok()?,
        caps.get(2)?.as_str().parse().ok()?,
        caps.get(3)?.as_str().parse().ok()?,
    ))
}

pub fn parse_title_detail(html: &str, id: &str) -> TitleDetail {
    let doc = Html::parse_document(html);
    let p_sel = sel("p");
    let mut meta: Vec<(String, String)> = Vec::new();
    for p in doc.select(&p_sel) {
        let t = text_of(p);
        if let Some(caps) = META_LINE_RE.captures(&t) {
            let key = caps.get(1).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
            let value = caps.get(2).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
            if !key.is_empty() && !meta.iter().any(|(k, _)| k == &key) {
                meta.push((key, value));
            }
        }
    }
    let get = |key: &str| -> Option<String> {
        meta.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };

    let title_text = Html::parse_document(html)
        .select(&sel("title"))
        .next()
        .map(text_of)
        .unwrap_or_default();
    let name = title_text
        .replace(" | hShop", "")
        .replace('\u{1F40E}', "")
        .trim()
        .to_string();

    let description = doc
        .select(&sel(".description-container"))
        .next()
        .map(text_of)
        .filter(|d| !d.is_empty());

    let size_text = get("Size");
    TitleDetail {
        id: id.to_string(),
        url: format!("{BASE}/t/{id}"),
        name,
        size_bytes: size_text.as_deref().and_then(size_to_bytes),
        size_text,
        product_code: get("Product Code"),
        content_type: get("Content Type"),
        title_id: get("Title ID"),
        version: get("Version"),
        added: get("Added"),
        updated: get("Updated"),
        downloads: get("Downloads"),
        seed: get("Seed"),
        sha256: get("SHA-256 Hash").or_else(|| get("SHA-256 hash")),
        description,
        needs_captcha: html.contains("cf-turnstile"),
    }
}

/// Parses the widget HTML that `/t/{id}/download-widget?captcha_token=…`
/// returns. The live path reads the rendered anchor from the verification
/// webview instead, so this stays as the reference parser and fallback —
/// covered by `parses_download_widget` in the tests below.
#[allow(dead_code)]
pub fn parse_download_widget(widget_html: &str) -> DownloadLinks {
    let doc = Html::parse_document(widget_html);
    let a_sel = sel("a");
    let mut links = DownloadLinks::default();

    for a in doc.select(&a_sel) {
        let label = text_of(a);
        let Some(href) = a.value().attr("href") else { continue };
        if href.is_empty() {
            continue;
        }
        let lower = label.to_ascii_lowercase();
        let href_lower = href.to_ascii_lowercase();
        if lower.contains("direct download") {
            links.direct = Some(href.to_string());
        } else if lower.contains("qr") || href_lower.contains("qr") {
            if links.qr.is_none() {
                links.qr = Some(href.to_string());
            }
        } else if lower.contains("install") {
            links.installer = Some(href.to_string());
        }
    }

    if let Some(img) = doc.select(&sel(r#"img[src*="qr"]"#)).next() {
        if let Some(src) = img.value().attr("src") {
            links.qr_image = Some(if src.starts_with("http") {
                src.to_string()
            } else {
                format!("{BASE}{src}")
            });
        }
    }
    links
}

// ── endpoints ────────────────────────────────────────────────────────────────

pub async fn fetch_subcategories(category_id: &str) -> Result<Vec<Subcategory>, String> {
    let html = fetch_text(&format!("{BASE}/c/{category_id}")).await?;
    Ok(parse_subcategory_rows(&html, category_id))
}

pub async fn fetch_titles(q: &TitlesQuery) -> Result<TitlePage, String> {
    let category = q.category.clone().unwrap_or_else(|| "games".into());
    let count = q.count.unwrap_or(50).clamp(10, 100);
    let offset = q.offset.unwrap_or(0).max(0);

    let path = match &q.subcategory {
        Some(sub) if !sub.is_empty() => format!("{BASE}/c/{category}/s/{sub}"),
        _ => format!("{BASE}/c/{category}"),
    };

    let mut params: Vec<(String, String)> = vec![
        ("count".into(), count.to_string()),
        ("offset".into(), offset.to_string()),
    ];
    if let Some(sb) = q.sort_by.as_deref() {
        if sb != "none" && !sb.is_empty() {
            params.push(("sb".into(), sb.to_string()));
        }
    }
    if let Some(sd) = q.direction.as_deref() {
        if sd != "none" && !sd.is_empty() {
            let dir = if sd == "desc" { "descending" } else { "ascending" };
            params.push(("sd".into(), dir.to_string()));
        }
    }
    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencode(v)))
        .collect();
    let url = format!("{path}?{}", query.join("&"));

    let html = fetch_text(&url).await?;
    Ok(TitlePage {
        titles: parse_title_rows(&html),
        total: parse_showing(&html).map(|(_, _, total)| total),
        offset,
        count,
    })
}

pub async fn search_titles(q: &SearchQuery) -> Result<TitlePage, String> {
    let count = q.count.unwrap_or(50).clamp(1, 100);
    let offset = q.offset.unwrap_or(0).max(0);
    let url = build_search_url(q);
    let html = fetch_text(&url).await?;

    Ok(TitlePage {
        titles: parse_title_rows(&html),
        total: parse_showing(&html).map(|(_, _, total)| total),
        offset,
        count,
    })
}

/// Builds the `/search/results` URL. Parameters mirror the site's own advanced
/// search: `q`, `qt`, `i` (include filters), `e` (exclude filters), `p`
/// (content type), `sb` (sort by) and `sd` (direction).
pub fn build_search_url(q: &SearchQuery) -> String {
    let query_text = q.query.clone().unwrap_or_default();
    let qt = q.qt.clone().unwrap_or_else(|| "Text".into());
    let count = q.count.unwrap_or(50).clamp(1, 100);
    let offset = q.offset.unwrap_or(0).max(0);

    let mut params = vec![
        ("q".to_string(), urlencode(&query_text)),
        ("qt".to_string(), urlencode(&qt)),
    ];

    if offset > 0 || count != 50 {
        params.push(("count".into(), count.to_string()));
        params.push(("offset".into(), offset.to_string()));
    }

    // `i=games.usa,dlc.japan` — dots and commas stay literal, the site splits
    // them after decoding; text is encoded per component. A whole category is
    // `games.*` (the site expands `*` to every subcategory).
    let filter_param = |raw: &str| -> Option<String> {
        let cleaned: Vec<String> = raw
            .split(',')
            .map(|pair| {
                let trimmed = pair.trim();
                if trimmed.is_empty() || trimmed == "none" {
                    return String::new();
                }
                if trimmed.contains('.') {
                    trimmed
                        .split('.')
                        .map(|part| urlencode(part.trim()))
                        .collect::<Vec<_>>()
                        .join(".")
                } else {
                    format!("{}.*", urlencode(trimmed))
                }
            })
            .filter(|pair| !pair.is_empty())
            .collect();
        if cleaned.is_empty() {
            None
        } else {
            Some(cleaned.join(","))
        }
    };

    if let Some(include) = q.include.as_deref().and_then(filter_param) {
        params.push(("i".into(), include));
    }
    if let Some(exclude) = q.exclude.as_deref().and_then(filter_param) {
        params.push(("e".into(), exclude));
    }
    if let Some(kind) = q.content_type.as_deref() {
        if kind != "none" && !kind.is_empty() {
            params.push(("p".into(), urlencode(kind)));
        }
    }
    if let Some(sb) = q.sort_by.as_deref() {
        if sb != "none" && !sb.is_empty() {
            params.push(("sb".into(), urlencode(sb)));
        }
    }
    if let Some(sd) = q.direction.as_deref() {
        if sd != "none" && !sd.is_empty() {
            params.push(("sd".into(), urlencode(sd)));
        }
    }

    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    format!("{BASE}/search/results?{}", query.join("&"))
}

pub async fn fetch_title_detail(id: &str) -> Result<TitleDetail, String> {
    let html = fetch_text(&format!("{BASE}/t/{id}")).await?;
    Ok(parse_title_detail(&html, id))
}

/// Minimal percent-encoding for query values (spaces, unicode, reserved).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
    <a class="list-entry" href="/t/1234">
      <div class="base-info"><h3>Zelda Ocarina of Time</h3><h4>content in Games › USA</h4></div>
      <div class="meta-content"><span>128.5 MiB</span><span>Size</span></div>
      <div class="meta-content"><span>0004000000033500</span><span>Title ID</span></div>
      <div class="meta-content"><span>CTR-P-AZEE</span><span>Product Code</span></div>
      <div class="meta-content"><span>1.2.0</span><span>Version</span></div>
    </a>
    "#;

    #[test]
    fn parses_title_rows() {
        let rows = parse_title_rows(FIXTURE);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.id, "1234");
        assert_eq!(r.name, "Zelda Ocarina of Time");
        assert_eq!(r.category_path, "Games › USA");
        assert_eq!(r.title_id.as_deref(), Some("0004000000033500"));
        assert_eq!(r.product_code.as_deref(), Some("CTR-P-AZEE"));
        assert_eq!(r.version.as_deref(), Some("1.2.0"));
        assert_eq!(r.size_bytes, Some((128.5f64 * 1024.0 * 1024.0).round() as i64));
    }

    #[test]
    fn parses_showing_counter() {
        let html = "<div><p>Showing <b>1</b>\u{2013}25 of 92</p></div>";
        assert_eq!(parse_showing(html), Some((1, 25, 92)));
    }

    #[test]
    fn parses_download_widget() {
        let html = r#"
          <a href="https://cdn.example/a.cia">Direct Download</a>
          <a href="https://cdn.example/a.png">QR</a>
          <a href="https://cdn.example/i.zip">Install</a>
        "#;
        let links = parse_download_widget(html);
        assert_eq!(links.direct.as_deref(), Some("https://cdn.example/a.cia"));
        assert_eq!(links.installer.as_deref(), Some("https://cdn.example/i.zip"));
    }

    // Live checks against hshop.erista.me — run with: cargo test -- --ignored
    #[test]
    #[ignore]
    fn live_subcategories() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let subs = rt.block_on(fetch_subcategories("games")).expect("fetch");
        assert!(!subs.is_empty(), "no subcategories parsed");
        println!(
            "subcategories: {} — first: {:?}",
            subs.len(),
            subs.first().map(|s| (&s.name, s.titles))
        );
    }

    #[test]
    #[ignore]
    fn live_search() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let page = rt
            .block_on(search_titles(&SearchQuery {
                query: Some("zelda".into()),
                ..Default::default()
            }))
            .expect("search");
        assert!(!page.titles.is_empty(), "no search results parsed");
        println!("search total={:?} rows={}", page.total, page.titles.len());
    }

    #[test]
    #[ignore]
    fn live_titles_and_detail() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let page = rt
            .block_on(fetch_titles(&TitlesQuery {
                category: Some("games".into()),
                ..Default::default()
            }))
            .expect("titles");
        assert!(!page.titles.is_empty(), "no titles parsed");
        let first = &page.titles[0];
        let detail = rt.block_on(fetch_title_detail(&first.id)).expect("detail");
        println!(
            "detail: {} sha={:?} size={:?} version={:?}",
            detail.name, detail.sha256, detail.size_text, detail.version
        );
        assert!(!detail.name.is_empty());
    }

    #[test]
    fn builds_search_url_with_filters() {
        let q = SearchQuery {
            query: Some("zelda".into()),
            include: Some("games.usa,virtual-console.japan".into()),
            exclude: Some("themes".into()),
            content_type: Some("piratelegit".into()),
            sort_by: Some("size".into()),
            direction: Some("descending".into()),
            ..Default::default()
        };
        let url = build_search_url(&q);
        assert!(url.contains("q=zelda"), "{url}");
        assert!(url.contains("i=games.usa,virtual-console.japan"), "{url}");
        assert!(url.contains("e=themes"), "{url}");
        assert!(url.contains("p=piratelegit"), "{url}");
        assert!(url.contains("sb=size"), "{url}");
        assert!(url.contains("sd=descending"), "{url}");
    }

    #[test]
    fn omits_empty_filters() {
        let q = SearchQuery {
            query: Some("mario".into()),
            include: Some("none".into()),
            content_type: Some("none".into()),
            sort_by: Some("none".into()),
            ..Default::default()
        };
        let url = build_search_url(&q);
        assert!(!url.contains("i="), "{url}");
        assert!(!url.contains("p="), "{url}");
        assert!(!url.contains("sb="), "{url}");
    }

    #[test]
    #[ignore]
    fn live_search_respects_filters() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let all = rt
            .block_on(search_titles(&SearchQuery {
                query: Some("zelda".into()),
                ..Default::default()
            }))
            .expect("search all");
        let filtered = rt
            .block_on(search_titles(&SearchQuery {
                query: Some("zelda".into()),
                include: Some("games.north-america".into()),
                ..Default::default()
            }))
            .expect("search filtered");
        println!("all={:?} filtered={:?}", all.total, filtered.total);
        assert!(filtered.total.unwrap_or(0) < all.total.unwrap_or(i64::MAX));
        assert!(filtered
            .titles
            .iter()
            .all(|t| t.category_path.to_lowercase().contains("games")));
    }

    #[test]
    #[ignore]
    fn live_update_code_search() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let page = rt
            .block_on(search_titles(&SearchQuery {
                query: Some("CTR-U-AXCE".into()),
                qt: Some("ProductCode".into()),
                ..Default::default()
            }))
            .expect("search update");
        let row = page
            .titles
            .iter()
            .find(|t| {
                t.product_code
                    .as_deref()
                    .map(|c| c.eq_ignore_ascii_case("CTR-U-AXCE"))
                    .unwrap_or(false)
            })
            .expect("update row");
        println!("update entry {} — {} ({:?})", row.id, row.name, row.version);
        assert!(row.version.is_some());
    }
}
