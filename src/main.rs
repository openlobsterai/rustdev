#[cfg(not(feature = "rust-dev"))]
compile_error!("This crate only supports the `rust-dev` feature (enabled by default).");

use actix_web::http::header::HeaderMap;
use actix_web::{
    http::{header, StatusCode},
    web, App, HttpRequest, HttpResponse, HttpServer,
};
use handlebars::{handlebars_helper, Handlebars, JsonValue};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, env, path::Path};
use tokio::fs;

const DEFAULT_PORT: u16 = 80;
const HTML_CONTENT_TYPE: &str = "text/html; charset=utf-8";
const HTML_CACHE_CONTROL: &str = "public, max-age=120, stale-while-revalidate=60";

const DATA_PATH: &str = "static/rustdev-hub-seed-v3.json";
const PROMO_PATH: &str = "static/promo.json";
const NIK_HTML_PATH: &str = "static/nik.html";

const NOT_FOUND_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>404 — rust.dev</title>
  <style>
    :root { color-scheme: dark; }
    body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: #0d1117; color: #c9d1d9; font: 14px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; }
    main { width: min(520px, calc(100% - 40px)); border: 1px solid #30363d; border-radius: 12px; background: #161b22; padding: 22px 24px; }
    h1 { margin: 0 0 6px; font-size: 18px; }
    p { margin: 0; color: #8b949e; }
    a { color: #58a6ff; text-decoration: none; }
    a:hover { text-decoration: underline; }
  </style>
</head>
<body>
  <main>
    <h1>404</h1>
    <p>Nothing here. Go <a href="/">home</a>.</p>
  </main>
</body>
</html>
"#;

const THELOBSTER_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>thelobster.ai — OpenLobster</title>
  <meta name="description" content="thelobster.ai — part of the OpenLobster ecosystem.">
  <meta property="og:title" content="thelobster.ai — OpenLobster">
  <meta property="og:description" content="thelobster.ai is now part of the OpenLobster ecosystem.">
  <meta property="og:type" content="website">
  <link rel="icon" href="data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><text y='.9em' font-size='90'>🦞</text></svg>">
  <style>
    :root { color-scheme: dark; }
    body {
      margin: 0;
      min-height: 100vh;
      display: grid;
      place-items: center;
      background: #0d1117;
      color: #c9d1d9;
      font: 14px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
    }
    main {
      width: min(720px, calc(100% - 40px));
      border: 1px solid #30363d;
      border-radius: 12px;
      background: #161b22;
      padding: 22px 24px;
    }
    h1 { margin: 0 0 8px; font-size: 22px; letter-spacing: -0.02em; }
    p { margin: 0 0 14px; color: #8b949e; }
    a { color: #58a6ff; text-decoration: none; }
    a:hover { text-decoration: underline; }
    .row { display: flex; flex-wrap: wrap; gap: 10px; margin-top: 14px; }
    a.btn {
      display: inline-block;
      padding: 8px 12px;
      border-radius: 8px;
      border: 1px solid #30363d;
      background: #0d1117;
      color: #c9d1d9;
      text-decoration: none;
      font-weight: 600;
    }
    a.btn:hover { border-color: #6e7681; text-decoration: none; }
    code {
      font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
      font-size: 13px;
      color: #c9d1d9;
    }
    .note { margin-top: 16px; font-size: 12px; color: #6e7681; }
  </style>
</head>
<body>
  <main>
    <h1>🦞 <span style="letter-spacing: -0.02em">thelobster.ai</span></h1>
    <p>This domain is now part of the <b>OpenLobster</b> ecosystem. More soon.</p>
    <div class="row">
      <a class="btn" href="https://openlobster.ai/">OpenLobster</a>
      <a class="btn" href="https://lobstermarket.ai/">LobsterMarket</a>
      <a class="btn" href="https://github.com/openlobsterai">GitHub</a>
    </div>
    <div class="note">You reached this via <code>rust.dev/thelobster</code>.</div>
  </main>
</body>
</html>
"#;

fn prefers_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .map(|accept| {
            accept.split(',').any(|item| {
                let trimmed = item.trim();
                trimmed == "application/json" || trimmed.ends_with("+json")
            })
        })
        .unwrap_or(false)
}

fn not_found_html() -> HttpResponse {
    HttpResponse::build(StatusCode::NOT_FOUND)
        .append_header((header::CONTENT_TYPE, HTML_CONTENT_TYPE))
        .body(NOT_FOUND_HTML)
}

fn not_found_for_request(req: &HttpRequest) -> HttpResponse {
    if prefers_json(req.headers()) {
        HttpResponse::NotFound().json(json!({"error": "Not found"}))
    } else {
        not_found_html()
    }
}

fn is_allowed_host(req: &HttpRequest) -> bool {
    let host = req.connection_info().host().to_ascii_lowercase();
    let hostname = host.split(':').next().unwrap_or(&host);
    matches!(hostname, "rust.dev" | "www.rust.dev" | "localhost") || hostname.starts_with("127.")
}

fn render_template_or_json(
    hb: &Handlebars<'_>,
    template: &str,
    context: &Value,
    req: &HttpRequest,
) -> HttpResponse {
    if prefers_json(req.headers()) {
        return HttpResponse::Ok().json(context);
    }

    match hb.render(template, context) {
        Ok(body) => HttpResponse::Ok()
            .append_header((header::CONTENT_TYPE, HTML_CONTENT_TYPE))
            .append_header((header::CACHE_CONTROL, HTML_CACHE_CONTROL))
            .body(body),
        Err(err) => {
            eprintln!("Template render error ({template}): {err}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Pages {
    #[serde(default)]
    tools: ToolPage,
    #[serde(default)]
    learn: LearnPage,
    #[serde(default)]
    work: WorkPage,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct ToolPage {
    #[serde(default)]
    categories: Vec<ToolCategory>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct LearnPage {
    #[serde(default)]
    tracks: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct WorkPage {
    #[serde(default)]
    job_sources: Vec<String>,
    #[serde(default)]
    role_archetypes: Vec<RoleArchetype>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ToolCategory {
    slug: String,
    title: String,
    #[serde(default)]
    items: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RoleArchetype {
    title: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct MediaItem {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct FeaturedMedia {
    #[serde(default)]
    youtube: Option<MediaItem>,
    #[serde(default)]
    twitter: Option<MediaItem>,
    #[serde(default)]
    x: Option<MediaItem>,
    #[serde(default)]
    article: Option<MediaItem>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct MediaAsset {
    #[serde(default)]
    logo_url: Option<String>,
    #[serde(default)]
    avatar_url: Option<String>,
    #[serde(default)]
    background_url: Option<String>,
    #[serde(default)]
    card_url: Option<String>,
    #[serde(default)]
    teaser_thumb_url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Updates {
    #[serde(default)]
    github_releases: Option<String>,
    #[serde(default)]
    github_tags: Option<String>,
    #[serde(default)]
    github_issues: Option<String>,
    #[serde(default)]
    site: Option<String>,
    #[serde(default)]
    twitter: Option<String>,
    #[serde(default)]
    youtube: Option<String>,
    #[serde(default)]
    schedule: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Ecosystem {
    slug: String,
    name: String,
    #[serde(default)]
    one_liner: String,
    #[serde(default)]
    featured_media: Option<FeaturedMedia>,
    #[serde(default)]
    media: Option<MediaAsset>,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    official_links: HashMap<String, String>,
    #[serde(default)]
    featured_tools: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Tool {
    slug: String,
    name: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    featured_media: Option<FeaturedMedia>,
    #[serde(default)]
    media: Option<MediaAsset>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    primary_label: Option<String>,
    #[serde(default)]
    tier: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    links: HashMap<String, String>,
    #[serde(default)]
    updates: Option<Updates>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Event {
    slug: String,
    title: String,
    #[serde(default)]
    href: Option<String>,
    #[serde(default)]
    teaser: Option<String>,
    #[serde(default)]
    schedule_note: Option<String>,
    #[serde(default)]
    featured_media: Option<FeaturedMedia>,
    #[serde(default)]
    media: Option<MediaAsset>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    primary_label: Option<String>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    starts_on: Option<String>,
    #[serde(default)]
    ends_on: Option<String>,
    #[serde(default)]
    location: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    updates: Option<Updates>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct LearningPath {
    slug: String,
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    featured_media: Option<FeaturedMedia>,
    #[serde(default)]
    media: Option<MediaAsset>,
    #[serde(default)]
    difficulty: String,
    #[serde(default)]
    duration_hours: u32,
    #[serde(default)]
    milestones: Vec<String>,
    #[serde(default)]
    resources: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct BestStart {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

impl BestStart {
    fn is_valid(&self) -> bool {
        self.title
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
            && self
                .url
                .as_ref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Creator {
    slug: String,
    name: String,
    #[serde(rename = "type")]
    r#type: String,
    #[serde(default)]
    featured_media: Option<FeaturedMedia>,
    #[serde(default)]
    media: Option<MediaAsset>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    primary_label: Option<String>,
    #[serde(default)]
    focus: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    links: HashMap<String, String>,
    #[serde(default)]
    about: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    video_id: Option<String>,
    #[serde(default)]
    best_start: Option<BestStart>,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    updates: Option<Updates>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PostLink {
    label: String,
    url: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct PostRelated {
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    events: Vec<String>,
    #[serde(default)]
    protocols: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Post {
    slug: String,
    title: String,
    #[serde(default)]
    featured_media: Option<FeaturedMedia>,
    #[serde(default)]
    about: Option<String>,
    #[serde(default)]
    media: Option<MediaAsset>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    primary_label: Option<String>,
    #[serde(default)]
    deck: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    published_on: String,
    #[serde(default)]
    author_handle: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    cover_image: Option<String>,
    #[serde(default)]
    links: Vec<PostLink>,
    #[serde(default)]
    body_md: String,
    #[serde(default)]
    sources: Vec<String>,
    #[serde(default)]
    related: Option<PostRelated>,
    #[serde(default)]
    updates: Option<Updates>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Resource {
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct JobSource {
    slug: String,
    name: String,
    url: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct JobCompany {
    #[serde(default)]
    name: String,
    #[serde(default)]
    domain: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Job {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    company: JobCompany,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    primary_label: Option<String>,
    #[serde(default)]
    about: String,
    #[serde(default)]
    apply_url: String,
    #[serde(default)]
    last_verified: Option<String>,
    #[serde(default)]
    media: Option<MediaAsset>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct PromoSlide {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    deck: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    published_on: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    href: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct PromoContent {
    #[serde(default)]
    slides: Vec<PromoSlide>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Label {
    slug: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Taxonomy {
    #[serde(default)]
    labels: Vec<Label>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct RustDevSeed {
    #[serde(default)]
    pages: Pages,
    #[serde(default, alias = "protocols")]
    ecosystems: Vec<Ecosystem>,
    #[serde(default)]
    tools: Vec<Tool>,
    #[serde(default)]
    events: Vec<Event>,
    #[serde(default)]
    learning_paths: Vec<LearningPath>,
    #[serde(default)]
    creators: Vec<Creator>,
    #[serde(default, alias = "news")]
    posts: Vec<Post>,
    #[serde(default)]
    resources: Vec<Resource>,
    #[serde(default, alias = "jobs_sources")]
    job_sources: Vec<JobSource>,
    #[serde(default)]
    taxonomy: Taxonomy,
    #[serde(default)]
    jobs: Vec<Job>,
}

#[derive(Clone)]
struct RustDevContent {
    ecosystems: Vec<Ecosystem>,
    tools: Vec<Tool>,
    events: Vec<Event>,
    learning_paths: Vec<LearningPath>,
    creators: Vec<Creator>,
    posts: Vec<Post>,
    resources: HashMap<String, Resource>,
    job_sources: HashMap<String, JobSource>,
    jobs: Vec<Job>,
    tool_categories: Vec<ToolCategory>,
    learn_tracks: Vec<String>,
    role_archetypes: Vec<RoleArchetype>,
    job_source_slugs: Vec<String>,
    labels: Vec<Label>,
    tools_index: HashMap<String, usize>,
    ecosystems_index: HashMap<String, usize>,
    events_index: HashMap<String, usize>,
    learning_index: HashMap<String, usize>,
    creators_index: HashMap<String, usize>,
    posts_index: HashMap<String, usize>,
}

fn build_index<T, F>(items: &[T], key: F) -> HashMap<String, usize>
where
    F: Fn(&T) -> &str,
{
    items
        .iter()
        .enumerate()
        .map(|(idx, item)| (key(item).to_owned(), idx))
        .collect()
}

fn slugify(input: &str) -> String {
    let mut slug = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() || ch == '-' || ch == '_' || ch == '/' {
            if !slug.ends_with('-') {
                slug.push('-');
            }
        }
    }
    slug.trim_matches('-').to_string()
}

fn derive_resource_slug(res: &Resource) -> Option<String> {
    if let Some(slug) = res.slug.clone() {
        return Some(slug);
    }
    if let Some(url) = res.url.split('/').rev().find(|segment| !segment.is_empty()) {
        let cleaned = url.split('.').next().unwrap_or(url);
        if !cleaned.is_empty() {
            return Some(slugify(cleaned));
        }
    }
    if !res.title.is_empty() {
        return Some(slugify(&res.title));
    }
    None
}

impl RustDevContent {
    fn from_seed(seed: RustDevSeed) -> Self {
        let tools_index = build_index(&seed.tools, |t| t.slug.as_str());
        let ecosystems_index = build_index(&seed.ecosystems, |e| e.slug.as_str());
        let events_index = build_index(&seed.events, |e| e.slug.as_str());
        let learning_index = build_index(&seed.learning_paths, |p| p.slug.as_str());
        let creators_index = build_index(&seed.creators, |c| c.slug.as_str());
        let posts_index = build_index(&seed.posts, |p| p.slug.as_str());

        let mut resources = HashMap::new();
        for mut res in seed.resources {
            let slug = derive_resource_slug(&res);
            if let Some(slug_val) = slug {
                res.slug = Some(slug_val.clone());
                resources.insert(slug_val, res);
            }
        }

        let job_sources = seed
            .job_sources
            .into_iter()
            .map(|src| (src.slug.clone(), src))
            .collect();

        Self {
            ecosystems: seed.ecosystems,
            tools: seed.tools,
            events: seed.events,
            learning_paths: seed.learning_paths,
            creators: seed.creators,
            posts: seed.posts,
            resources,
            job_sources,
            jobs: seed.jobs,
            tool_categories: seed.pages.tools.categories,
            learn_tracks: seed.pages.learn.tracks,
            role_archetypes: seed.pages.work.role_archetypes,
            job_source_slugs: seed.pages.work.job_sources,
            labels: seed.taxonomy.labels,
            tools_index,
            ecosystems_index,
            events_index,
            learning_index,
            creators_index,
            posts_index,
        }
    }

    fn ecosystem_by_slug(&self, slug: &str) -> Option<&Ecosystem> {
        self.ecosystems_index
            .get(slug)
            .and_then(|idx| self.ecosystems.get(*idx))
    }

    fn tool_by_slug(&self, slug: &str) -> Option<&Tool> {
        self.tools_index
            .get(slug)
            .and_then(|idx| self.tools.get(*idx))
    }

    fn event_by_slug(&self, slug: &str) -> Option<&Event> {
        self.events_index
            .get(slug)
            .and_then(|idx| self.events.get(*idx))
    }

    fn learning_path_by_slug(&self, slug: &str) -> Option<&LearningPath> {
        self.learning_index
            .get(slug)
            .and_then(|idx| self.learning_paths.get(*idx))
    }

    fn creator_by_slug(&self, slug: &str) -> Option<&Creator> {
        self.creators_index
            .get(slug)
            .and_then(|idx| self.creators.get(*idx))
    }

    fn post_by_slug(&self, slug: &str) -> Option<&Post> {
        self.posts_index
            .get(slug)
            .and_then(|idx| self.posts.get(*idx))
    }

    fn tools_for(&self, slugs: &[String]) -> Vec<Tool> {
        slugs
            .iter()
            .filter_map(|slug| self.tool_by_slug(slug))
            .cloned()
            .collect()
    }

    fn resources_for(&self, slugs: &[String]) -> Vec<Resource> {
        slugs
            .iter()
            .filter_map(|slug| self.resources.get(slug).cloned())
            .collect()
    }

    fn job_sources_for(&self, slugs: &[String]) -> Vec<JobSource> {
        slugs
            .iter()
            .filter_map(|slug| self.job_sources.get(slug).cloned())
            .collect()
    }

    fn job_sources_in_order(&self) -> Vec<JobSource> {
        let ordered = self.job_sources_for(&self.job_source_slugs);
        if !ordered.is_empty() {
            return ordered;
        }

        let mut fallback: Vec<JobSource> = self.job_sources.values().cloned().collect();
        fallback.sort_by(|a, b| a.name.cmp(&b.name));
        fallback
    }

    fn learning_paths_for_tracks(&self) -> Vec<LearningPath> {
        if self.learn_tracks.is_empty() {
            return self.learning_paths.clone();
        }

        let ordered: Vec<LearningPath> = self
            .learn_tracks
            .iter()
            .filter_map(|slug| self.learning_path_by_slug(slug))
            .cloned()
            .collect();

        if ordered.is_empty() {
            self.learning_paths.clone()
        } else {
            ordered
        }
    }
}

async fn load_rust_dev_content(path: impl AsRef<Path>) -> std::io::Result<RustDevContent> {
    let bytes = fs::read(path.as_ref()).await?;
    let seed: RustDevSeed = serde_json::from_slice(&bytes)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    Ok(RustDevContent::from_seed(seed))
}

async fn load_promo_content(path: impl AsRef<Path>) -> std::io::Result<PromoContent> {
    let bytes = fs::read(path.as_ref()).await?;
    let content: PromoContent = serde_json::from_slice(&bytes)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    Ok(content)
}

fn build_handlebars() -> std::io::Result<Handlebars<'static>> {
    let mut hb = Handlebars::new();

    hb.register_template_file("index-rust", "static/index_rust_dev.html")
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    hb.register_template_file(
        "ecosystems-list",
        "static/rustdev/templates/ecosystems-list.html",
    )
    .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    hb.register_template_file(
        "ecosystem-single",
        "static/rustdev/templates/ecosystem-single.html",
    )
    .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    hb.register_template_file("tools-list", "static/rustdev/templates/tools-list.html")
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    hb.register_template_file("tool-single", "static/rustdev/templates/tool-single.html")
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    hb.register_template_file("events-list", "static/rustdev/templates/events-list.html")
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    hb.register_template_file("event-single", "static/rustdev/templates/event-single.html")
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    hb.register_template_file("learn-list", "static/rustdev/templates/learn-list.html")
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    hb.register_template_file(
        "learning-single",
        "static/rustdev/templates/learning-single.html",
    )
    .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    hb.register_template_file(
        "creators-list",
        "static/rustdev/templates/creators-list.html",
    )
    .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    hb.register_template_file(
        "creator-single",
        "static/rustdev/templates/creator-single.html",
    )
    .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    hb.register_template_file("news-list", "static/rustdev/templates/news-list.html")
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    hb.register_template_file("post-single", "static/rustdev/templates/post-single.html")
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    hb.register_template_file("jobs-list", "static/rustdev/templates/jobs-list.html")
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    hb.register_template_file(
        "component/youtube-embed",
        "static/rustdev/templates/components/youtube-embed.html",
    )
    .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    hb.register_template_file(
        "component/twitter-embed",
        "static/rustdev/templates/components/twitter-embed.html",
    )
    .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;

    handlebars_helper!(eq: |a: JsonValue, b: JsonValue| a == b);
    hb.register_helper("eq", Box::new(eq));

    Ok(hb)
}

fn extract_youtube_id(url: &str) -> Option<String> {
    if let Some(id) = url.split("youtu.be/").nth(1) {
        return Some(id.split(&['?', '&', '#'][..]).next()?.to_string());
    }
    if let Some(pos) = url.find("watch?v=") {
        return Some(
            url[(pos + 8)..]
                .split(&['&', '#'][..])
                .next()
                .unwrap_or("")
                .to_string(),
        );
    }
    if let Some(pos) = url.find("embed/") {
        return Some(
            url[(pos + 6)..]
                .split(&['?', '&', '#'][..])
                .next()
                .unwrap_or("")
                .to_string(),
        );
    }
    None
}

struct EmbedFragments {
    youtube: Option<String>,
    twitter: Option<String>,
    has_twitter: bool,
    section_title: Option<String>,
}

fn build_embed_fragments(hb: &Handlebars<'_>, media: Option<&FeaturedMedia>) -> EmbedFragments {
    let mut youtube_html = None;
    let mut twitter_html = None;
    let mut title = None;

    if let Some(media) = media {
        if let Some(yt) = media.youtube.as_ref().and_then(|m| m.url.as_ref()) {
            if let Some(id) = extract_youtube_id(yt) {
                let video_title = media.youtube.as_ref().and_then(|m| m.title.clone());
                if title.is_none() {
                    title = video_title.clone();
                }
                if let Ok(html) = hb.render(
                    "component/youtube-embed",
                    &json!({ "video_id": id, "video_title": video_title }),
                ) {
                    youtube_html = Some(html);
                }
            }
        }

        let twitter_url = media
            .twitter
            .as_ref()
            .and_then(|m| m.url.as_ref())
            .or_else(|| media.x.as_ref().and_then(|m| m.url.as_ref()))
            .cloned();
        if let Some(url) = twitter_url {
            if title.is_none() {
                title = media
                    .twitter
                    .as_ref()
                    .and_then(|m| m.title.clone())
                    .or_else(|| media.x.as_ref().and_then(|m| m.title.clone()));
            }
            if let Ok(html) = hb.render("component/twitter-embed", &json!({ "tweet_url": url })) {
                twitter_html = Some(html);
            }
        }
    }

    EmbedFragments {
        youtube: youtube_html,
        has_twitter: twitter_html.is_some(),
        twitter: twitter_html,
        section_title: title,
    }
}

fn promo_slide_to_value(slide: &PromoSlide) -> Value {
    json!({
        "type": slide.r#type,
        "slug": slide.slug,
        "title": slide.title,
        "deck": slide.deck,
        "kind": if slide.kind.is_empty() { slide.r#type.clone() } else { slide.kind.clone() },
        "published_on": slide.published_on,
        "tags": slide.tags,
        "href": slide.href,
    })
}

fn build_carousel_items(promo: &PromoContent, rustdev: &RustDevContent) -> Vec<Value> {
    let mut items = Vec::new();

    // Promo slides go first (pinned to top of carousel).
    for slide in &promo.slides {
        items.push(promo_slide_to_value(slide));
    }

    // Then latest news.
    for post in rustdev.posts.iter().take(4) {
        items.push(json!({
            "type": "news",
            "slug": post.slug,
            "title": post.title,
            "deck": post.deck,
            "kind": post.kind.clone(),
            "published_on": post.published_on,
            "tags": post.tags,
        }));
    }

    // Add upcoming events.
    for event in rustdev
        .events
        .iter()
        .filter(|e| e.status == "upcoming")
        .take(3)
    {
        let deck = format!(
            "{}{}{}",
            event.location,
            if event.location.is_empty() {
                ""
            } else {
                " • "
            },
            event
                .starts_on
                .clone()
                .unwrap_or_else(|| "Date TBA".to_string())
        );
        items.push(json!({
            "type": "events",
            "slug": event.slug,
            "title": event.title,
            "deck": deck,
            "kind": "event",
            "published_on": event.starts_on.clone().unwrap_or_default(),
            "tags": event.tags,
        }));
    }

    items.truncate(7);
    items
}

fn render_rust_home(
    hb: &Handlebars<'_>,
    promo: &PromoContent,
    rustdev: &RustDevContent,
) -> HttpResponse {
    let carousel_items = build_carousel_items(promo, rustdev);
    let context = json!({ "carousel_items": carousel_items });

    match hb.render("index-rust", &context) {
        Ok(body) => HttpResponse::Ok()
            .append_header((header::CONTENT_TYPE, HTML_CONTENT_TYPE))
            .append_header((header::CACHE_CONTROL, HTML_CACHE_CONTROL))
            .body(body),
        Err(err) => {
            eprintln!("Failed to render rust.dev home: {err}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

async fn index(
    rustdev: web::Data<RustDevContent>,
    hb: web::Data<Handlebars<'_>>,
    promo: web::Data<PromoContent>,
    req: HttpRequest,
) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }
    render_rust_home(&hb, &promo, &rustdev)
}

async fn thelobster(req: HttpRequest) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }

    if prefers_json(req.headers()) {
        return HttpResponse::Ok().json(json!({
            "slug": "thelobster",
            "title": "thelobster.ai",
            "links": {
                "openlobster": "https://openlobster.ai/",
                "lobstermarket": "https://lobstermarket.ai/",
                "github": "https://github.com/openlobsterai",
            }
        }));
    }

    HttpResponse::Ok()
        .append_header((header::CONTENT_TYPE, HTML_CONTENT_TYPE))
        .append_header((header::CACHE_CONTROL, HTML_CACHE_CONTROL))
        .body(THELOBSTER_HTML)
}

#[derive(Clone)]
struct ProfilePages {
    nik: String,
}

async fn load_profile_pages() -> std::io::Result<ProfilePages> {
    let nik = fs::read_to_string(NIK_HTML_PATH).await?;
    Ok(ProfilePages { nik })
}

async fn profile_page(pages: web::Data<ProfilePages>, req: HttpRequest) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }

    HttpResponse::Ok()
        .append_header((header::CONTENT_TYPE, HTML_CONTENT_TYPE))
        .append_header((header::CACHE_CONTROL, HTML_CACHE_CONTROL))
        .body(pages.nik.clone())
}

async fn rustdev_ecosystems_list(
    rustdev: web::Data<RustDevContent>,
    hb: web::Data<Handlebars<'_>>,
    req: HttpRequest,
) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }
    let context = json!({ "ecosystems": rustdev.ecosystems.clone() });
    render_template_or_json(&hb, "ecosystems-list", &context, &req)
}

async fn rustdev_ecosystem_page(
    slug: web::Path<String>,
    rustdev: web::Data<RustDevContent>,
    hb: web::Data<Handlebars<'_>>,
    req: HttpRequest,
) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }
    if let Some(ecosystem) = rustdev.ecosystem_by_slug(slug.as_str()).cloned() {
        let embeds = build_embed_fragments(&hb, ecosystem.featured_media.as_ref());
        let media = if embeds.youtube.is_some() || embeds.twitter.is_some() {
            Some(json!({
                "section_title": embeds.section_title.unwrap_or_else(|| "Featured media".to_string())
            }))
        } else {
            None
        };
        let context = json!({
            "slug": ecosystem.slug,
            "name": ecosystem.name,
            "one_liner": ecosystem.one_liner,
            "topics": ecosystem.topics,
            "official_links": ecosystem.official_links,
            "featured_tools": rustdev.tools_for(&ecosystem.featured_tools),
            "media": media,
            "embed_youtube": embeds.youtube,
            "embed_twitter": embeds.twitter,
            "has_twitter": embeds.has_twitter,
        });
        return render_template_or_json(&hb, "ecosystem-single", &context, &req);
    }

    not_found_for_request(&req)
}

async fn rustdev_tools_list(
    rustdev: web::Data<RustDevContent>,
    hb: web::Data<Handlebars<'_>>,
    req: HttpRequest,
) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }

    let categories: Vec<Value> = if rustdev.tool_categories.is_empty() {
        vec![json!({
            "title": "All tools",
            "slug": "all",
            "tools": rustdev.tools.clone(),
        })]
    } else {
        rustdev
            .tool_categories
            .iter()
            .map(|cat| {
                let tools = cat
                    .items
                    .iter()
                    .filter_map(|slug| rustdev.tool_by_slug(slug))
                    .cloned()
                    .collect::<Vec<_>>();
                json!({
                    "title": cat.title,
                    "slug": cat.slug,
                    "tools": tools,
                })
            })
            .collect()
    };

    let context = json!({
        "categories": categories,
        "labels": rustdev.labels,
    });
    render_template_or_json(&hb, "tools-list", &context, &req)
}

async fn rustdev_tool_page(
    slug: web::Path<String>,
    rustdev: web::Data<RustDevContent>,
    hb: web::Data<Handlebars<'_>>,
    req: HttpRequest,
) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }

    if let Some(tool) = rustdev.tool_by_slug(slug.as_str()).cloned() {
        let embeds = build_embed_fragments(&hb, tool.featured_media.as_ref());
        let media = if embeds.youtube.is_some() || embeds.twitter.is_some() {
            Some(json!({
                "section_title": embeds.section_title.unwrap_or_else(|| "Featured media".to_string())
            }))
        } else {
            None
        };
        let mut context = serde_json::to_value(tool).unwrap_or_else(|_| json!({}));
        context["media"] = media.unwrap_or(Value::Null);
        context["embed_youtube"] = embeds.youtube.map(Value::String).unwrap_or(Value::Null);
        context["embed_twitter"] = embeds.twitter.map(Value::String).unwrap_or(Value::Null);
        context["has_twitter"] = Value::Bool(embeds.has_twitter);
        return render_template_or_json(&hb, "tool-single", &context, &req);
    }

    not_found_for_request(&req)
}

async fn rustdev_events_list(
    rustdev: web::Data<RustDevContent>,
    hb: web::Data<Handlebars<'_>>,
    req: HttpRequest,
) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }

    let mut upcoming = Vec::new();
    let mut past = Vec::new();

    for event in &rustdev.events {
        if event.status == "past" {
            past.push(event.clone());
        } else {
            upcoming.push(event.clone());
        }
    }

    let context = json!({
        "upcoming": upcoming,
        "past": past,
        "labels": rustdev.labels,
    });
    render_template_or_json(&hb, "events-list", &context, &req)
}

async fn rustdev_event_page(
    slug: web::Path<String>,
    rustdev: web::Data<RustDevContent>,
    hb: web::Data<Handlebars<'_>>,
    req: HttpRequest,
) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }

    if let Some(event) = rustdev.event_by_slug(slug.as_str()).cloned() {
        let embeds = build_embed_fragments(&hb, event.featured_media.as_ref());
        let media = if embeds.youtube.is_some() || embeds.twitter.is_some() {
            Some(json!({
                "section_title": embeds.section_title.unwrap_or_else(|| "Featured media".to_string())
            }))
        } else {
            None
        };
        let mut context = serde_json::to_value(event).unwrap_or_else(|_| json!({}));
        context["media"] = media.unwrap_or(Value::Null);
        context["embed_youtube"] = embeds.youtube.map(Value::String).unwrap_or(Value::Null);
        context["embed_twitter"] = embeds.twitter.map(Value::String).unwrap_or(Value::Null);
        context["has_twitter"] = Value::Bool(embeds.has_twitter);
        return render_template_or_json(&hb, "event-single", &context, &req);
    }

    not_found_for_request(&req)
}

async fn rustdev_learn_list(
    rustdev: web::Data<RustDevContent>,
    hb: web::Data<Handlebars<'_>>,
    req: HttpRequest,
) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }

    let sections = vec![json!({
        "title": "Learning paths",
        "paths": rustdev.learning_paths_for_tracks(),
    })];
    let context = json!({ "sections": sections });
    render_template_or_json(&hb, "learn-list", &context, &req)
}

async fn rustdev_learning_page(
    slug: web::Path<String>,
    rustdev: web::Data<RustDevContent>,
    hb: web::Data<Handlebars<'_>>,
    req: HttpRequest,
) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }

    if let Some(path) = rustdev.learning_path_by_slug(slug.as_str()).cloned() {
        let resources_data = rustdev.resources_for(&path.resources);
        let embeds = build_embed_fragments(&hb, path.featured_media.as_ref());
        let media = if embeds.youtube.is_some() || embeds.twitter.is_some() {
            Some(json!({
                "section_title": embeds.section_title.unwrap_or_else(|| "Featured media".to_string())
            }))
        } else {
            None
        };
        let context = json!({
            "slug": path.slug,
            "title": path.title,
            "summary": path.summary,
            "difficulty": path.difficulty,
            "duration_hours": path.duration_hours,
            "milestones": path.milestones,
            "resources_data": resources_data,
            "media": media,
            "embed_youtube": embeds.youtube,
            "embed_twitter": embeds.twitter,
            "has_twitter": embeds.has_twitter,
        });
        return render_template_or_json(&hb, "learning-single", &context, &req);
    }

    not_found_for_request(&req)
}

fn creator_section_title(kind: &str) -> String {
    match kind {
        "youtube" => "YouTube".to_string(),
        "podcast" => "Podcasts".to_string(),
        "newsletter" => "Newsletters".to_string(),
        "playlist" => "Playlists".to_string(),
        other => {
            let mut chars = other.chars();
            if let Some(first) = chars.next() {
                format!("{}{}", first.to_uppercase(), chars.as_str())
            } else {
                "Creators".to_string()
            }
        }
    }
}

async fn rustdev_creators_list(
    rustdev: web::Data<RustDevContent>,
    hb: web::Data<Handlebars<'_>>,
    req: HttpRequest,
) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }

    let mut grouped: HashMap<String, Vec<Creator>> = HashMap::new();
    for creator in &rustdev.creators {
        grouped
            .entry(creator.r#type.clone())
            .or_default()
            .push(creator.clone());
    }

    let mut sections: Vec<Value> = Vec::new();

    // Order: playlist -> youtube -> newsletter -> everything else (alphabetical).
    if let Some(creators) = grouped.remove("playlist") {
        sections.push(json!({
            "title": creator_section_title("playlist"),
            "creators": creators,
        }));
    }
    if let Some(creators) = grouped.remove("youtube") {
        sections.push(json!({
            "title": creator_section_title("youtube"),
            "creators": creators,
        }));
    }
    if let Some(creators) = grouped.remove("newsletter") {
        sections.push(json!({
            "title": creator_section_title("newsletter"),
            "creators": creators,
        }));
    }

    let mut remaining: Vec<_> = grouped.into_iter().collect();
    remaining.sort_by(|a, b| a.0.cmp(&b.0));
    for (kind, creators) in remaining {
        sections.push(json!({
            "title": creator_section_title(&kind),
            "creators": creators,
        }));
    }

    let context = json!({
        "sections": sections,
        "labels": rustdev.labels,
    });
    render_template_or_json(&hb, "creators-list", &context, &req)
}

async fn rustdev_creator_page(
    slug: web::Path<String>,
    rustdev: web::Data<RustDevContent>,
    hb: web::Data<Handlebars<'_>>,
    req: HttpRequest,
) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }

    if let Some(creator) = rustdev.creator_by_slug(slug.as_str()).cloned() {
        let embeds = build_embed_fragments(&hb, creator.featured_media.as_ref());
        let media = if embeds.youtube.is_some() || embeds.twitter.is_some() {
            Some(json!({
                "section_title": embeds.section_title.unwrap_or_else(|| "Featured media".to_string())
            }))
        } else {
            None
        };
        let best_start = creator.best_start.as_ref().and_then(|bs| {
            if bs.is_valid() {
                Some(bs.clone())
            } else {
                None
            }
        });
        let context = json!({
            "slug": creator.slug,
            "name": creator.name,
            "type": creator.r#type,
            "focus": creator.focus,
            "links": creator.links,
            "about": creator.about,
            "description": creator.description,
            "video_id": creator.video_id,
            "best_start": best_start,
            "thumbnail": creator.thumbnail,
            "media": media,
            "embed_youtube": embeds.youtube,
            "embed_twitter": embeds.twitter,
            "has_twitter": embeds.has_twitter,
        });
        return render_template_or_json(&hb, "creator-single", &context, &req);
    }

    not_found_for_request(&req)
}

async fn rustdev_news_list(
    rustdev: web::Data<RustDevContent>,
    hb: web::Data<Handlebars<'_>>,
    req: HttpRequest,
) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }

    let context = json!({
        "posts": rustdev.posts.clone(),
        "labels": rustdev.labels,
    });
    render_template_or_json(&hb, "news-list", &context, &req)
}

async fn rustdev_post_page(
    slug: web::Path<String>,
    rustdev: web::Data<RustDevContent>,
    hb: web::Data<Handlebars<'_>>,
    req: HttpRequest,
) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }

    if let Some(post) = rustdev.post_by_slug(slug.as_str()).cloned() {
        let embeds = build_embed_fragments(&hb, post.featured_media.as_ref());
        let media = if embeds.youtube.is_some() || embeds.twitter.is_some() {
            Some(json!({
                "section_title": embeds.section_title.unwrap_or_else(|| "Featured media".to_string())
            }))
        } else {
            None
        };
        let mut context = serde_json::to_value(post).unwrap_or_else(|_| json!({}));
        // Fallback body from about/deck if body_md is missing.
        if context
            .get("body_md")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
        {
            if let Some(about) = context.get("about").and_then(|v| v.as_str()) {
                context["body_md"] = Value::String(about.to_string());
            } else if let Some(deck) = context.get("deck").and_then(|v| v.as_str()) {
                context["body_md"] = Value::String(deck.to_string());
            }
        }
        context["media"] = media.unwrap_or(Value::Null);
        context["embed_youtube"] = embeds.youtube.map(Value::String).unwrap_or(Value::Null);
        context["embed_twitter"] = embeds.twitter.map(Value::String).unwrap_or(Value::Null);
        context["has_twitter"] = Value::Bool(embeds.has_twitter);
        return render_template_or_json(&hb, "post-single", &context, &req);
    }

    not_found_for_request(&req)
}

async fn rustdev_jobs_list(
    rustdev: web::Data<RustDevContent>,
    hb: web::Data<Handlebars<'_>>,
    req: HttpRequest,
) -> HttpResponse {
    if !is_allowed_host(&req) {
        return not_found_for_request(&req);
    }

    let context = json!({
        "job_sources": rustdev.job_sources_in_order(),
        "role_archetypes": rustdev.role_archetypes.clone(),
        "labels": rustdev.labels,
        "jobs": rustdev.jobs,
    });
    render_template_or_json(&hb, "jobs-list", &context, &req)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let rustdev_content = load_rust_dev_content(DATA_PATH).await?;
    let promo_content = load_promo_content(PROMO_PATH).await.unwrap_or_default();
    let handlebars = build_handlebars()?;
    let profile_pages = load_profile_pages().await?;

    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let addr: std::net::SocketAddr = format!("0.0.0.0:{port}")
        .parse()
        .expect("invalid listen addr");
    println!("listening on http://{addr}");

    let rustdev_data = web::Data::new(rustdev_content);
    let promo_data = web::Data::new(promo_content);
    let hb_data = web::Data::new(handlebars);
    let profile_pages_data = web::Data::new(profile_pages);

    HttpServer::new(move || {
        App::new()
            .app_data(rustdev_data.clone())
            .app_data(promo_data.clone())
            .app_data(hb_data.clone())
            .app_data(profile_pages_data.clone())
            .service(web::resource("/").route(web::get().to(index)))
            .service(web::resource("/index.html").route(web::get().to(index)))
            .service(web::resource("/thelobster").route(web::get().to(thelobster)))
            .service(web::resource("/thelobster/").route(web::get().to(thelobster)))
            .service(web::resource("/me").route(web::get().to(profile_page)))
            .service(web::resource("/me/").route(web::get().to(profile_page)))
            .service(web::resource("/nik").route(web::get().to(profile_page)))
            .service(web::resource("/nik/").route(web::get().to(profile_page)))
            .service(web::resource("/ecosystems").route(web::get().to(rustdev_ecosystems_list)))
            .service(
                web::resource("/ecosystems/{slug}").route(web::get().to(rustdev_ecosystem_page)),
            )
            .service(web::resource("/tools").route(web::get().to(rustdev_tools_list)))
            .service(web::resource("/tools/{slug}").route(web::get().to(rustdev_tool_page)))
            .service(web::resource("/events").route(web::get().to(rustdev_events_list)))
            .service(web::resource("/events/{slug}").route(web::get().to(rustdev_event_page)))
            .service(web::resource("/learn").route(web::get().to(rustdev_learn_list)))
            .service(web::resource("/learn/{slug}").route(web::get().to(rustdev_learning_page)))
            .service(web::resource("/creators").route(web::get().to(rustdev_creators_list)))
            .service(web::resource("/creators/{slug}").route(web::get().to(rustdev_creator_page)))
            .service(web::resource("/news").route(web::get().to(rustdev_news_list)))
            .service(web::resource("/news/{slug}").route(web::get().to(rustdev_post_page)))
            .service(web::resource("/jobs").route(web::get().to(rustdev_jobs_list)))
            .default_service(web::route().to(|| async { not_found_html() }))
    })
    .bind(addr)?
    .run()
    .await
}
