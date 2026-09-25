use chrono::{DateTime, TimeZone, Utc};
use log::{error, info, warn};
use parking_lot::RwLock;
use rand::{distributions::Alphanumeric, Rng};
use regex::Regex;
use reqwest::{header::LOCATION, redirect, Client, Proxy, Response, StatusCode};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::config::AppConfig;
use crate::models::instagram::{InstagramPost, InstagramReel, InstagramUser, InstagramUserStats};
use crate::proxy::ProxyManager;

const GRAPHQL_URL: &str = "https://www.instagram.com/api/graphql/";
const IG_APP_ID: &str = "936619743392459";
const POSTS_PAGE_SIZE: u32 = 12;

// Media ids encode their creation time as (ms since this epoch) << 23
const INSTAGRAM_EPOCH_MS: i64 = 1_314_220_021_721;
const MEDIA_ID_TIMESTAMP_SHIFT: u32 = 23;
const MEDIA_TYPE_VIDEO: u64 = 2;

const PROFILE_QUERY: &str = "PolarisLoggedOutDesktopWWWProfileRootContentQuery";
const POSTS_QUERY: &str = "PolarisLoggedOutDesktopWWWProfilePostsTabContentQuery";
// Observed 2026-09; refreshed from the profile page when Instagram rotates them
const DEFAULT_PROFILE_DOC_ID: &str = "27981003384861049";
const DEFAULT_POSTS_DOC_ID: &str = "27553725110923321";
const DOC_ID_REFRESH_COOLDOWN: Duration = Duration::from_secs(600);

#[derive(Error, Debug)]
pub enum ScraperError {
    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("Parsing error: {0}")]
    ParsingError(String),

    #[error("Rate limited or blocked")]
    RateLimited,

    #[error("Profile not found")]
    ProfileNotFound,

    #[error("Private profile")]
    PrivateProfile,

    #[error("Proxy error: {0}")]
    ProxyError(String),

    #[error("No proxies configured")]
    NoProxiesConfigured,

    #[error("All proxies failed")]
    AllProxiesFailed,

    #[error("Unauthorized access: {0}")]
    UnauthorizedAccess(String),

    #[error("Instagram error: {0}")]
    UpstreamError(String),
}

impl ScraperError {
    // Transient failures that a new proxy exit IP may get past
    fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::NetworkError(_) | Self::ProxyError(_) | Self::RateLimited | Self::AllProxiesFailed
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum Query {
    Profile,
    Posts,
}

impl Query {
    fn name(self) -> &'static str {
        match self {
            Query::Profile => PROFILE_QUERY,
            Query::Posts => POSTS_QUERY,
        }
    }

    fn variables(self, username: &str) -> Value {
        match self {
            Query::Profile => json!({ "username": username }),
            Query::Posts => json!({ "first": POSTS_PAGE_SIZE, "username": username }),
        }
    }
}

#[derive(Debug)]
struct DocIds {
    profile: String,
    posts: String,
    refreshed_at: Option<Instant>,
}

#[derive(Debug, PartialEq)]
enum GraphqlOutcome {
    User(Value),
    NotFound,
    StaleDocId,
    Rejected(String),
}

pub struct InstagramScraper {
    config: AppConfig,
    proxy_manager: ProxyManager,
    doc_ids: RwLock<DocIds>,
    refresh_lock: Mutex<()>,
}

impl InstagramScraper {
    pub fn new(config: AppConfig, proxy_manager: ProxyManager) -> Self {
        Self {
            config,
            proxy_manager,
            doc_ids: RwLock::new(DocIds {
                profile: DEFAULT_PROFILE_DOC_ID.to_string(),
                posts: DEFAULT_POSTS_DOC_ID.to_string(),
                refreshed_at: None,
            }),
            refresh_lock: Mutex::new(()),
        }
    }

    pub async fn scrape_user(&self, username: &str) -> Result<InstagramUser, ScraperError> {
        info!("Scraping Instagram user: {}", username);

        let proxy_url = self.select_proxy()?;
        let result = self.scrape_via_proxy(username, &proxy_url).await;

        if let Err(ScraperError::ProxyError(msg)) = &result {
            // A lone proxy is usually a rotating gateway: its next connection already gets a new exit IP
            if self.proxy_manager.get_proxy_count().1 > 1 {
                warn!("Proxy error: {}, marking proxy as unavailable", msg);
                self.proxy_manager.mark_proxy_unavailable(&proxy_url);
            }
        }

        result
    }

    /// Retries only transient failures; Instagram-side answers (not found, private, login wall) are returned as is
    pub async fn scrape_user_with_retry(&self, username: &str) -> Result<InstagramUser, ScraperError> {
        let mut attempt = 0;

        loop {
            match self.scrape_user(username).await {
                Ok(user) => return Ok(user),
                Err(err) if err.is_retryable() && attempt < self.config.max_retries => {
                    attempt += 1;
                    warn!("Scraping {} failed: {}. Retry {}/{}", username, err, attempt, self.config.max_retries);

                    if matches!(err, ScraperError::AllProxiesFailed) {
                        self.proxy_manager.reset_all_proxies();
                    }

                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(err) => {
                    error!("Failed to scrape {}: {}", username, err);
                    return Err(err);
                }
            }
        }
    }

    // Never falls back to a direct connection
    fn select_proxy(&self) -> Result<String, ScraperError> {
        let (_, total) = self.proxy_manager.get_proxy_count();
        if total == 0 {
            return Err(ScraperError::NoProxiesConfigured);
        }

        self.proxy_manager
            .get_random_proxy()
            .ok_or(ScraperError::AllProxiesFailed)
    }

    // One client per scrape so every request shares a connection, hence the same proxy exit IP
    fn build_client(&self, proxy_url: &str) -> Result<Client, ScraperError> {
        let proxy_url = self.proxy_manager.normalize_proxy_url(proxy_url);
        let proxy = Proxy::all(&proxy_url)
            .map_err(|e| ScraperError::ProxyError(format!("Failed to create proxy: {}", e)))?;

        Client::builder()
            .timeout(Duration::from_secs(self.config.timeout))
            .user_agent(&self.config.user_agent)
            .proxy(proxy)
            // Following a login redirect would download the login page through the proxy
            .redirect(redirect::Policy::none())
            .build()
            .map_err(|e| ScraperError::ProxyError(format!("Failed to build client: {}", e)))
    }

    async fn scrape_via_proxy(&self, username: &str, proxy_url: &str) -> Result<InstagramUser, ScraperError> {
        let client = self.build_client(proxy_url)?;

        let profile = self.query_user(&client, username, Query::Profile).await?;
        let mut user = parse_profile(&profile, username);
        if user.is_private {
            return Err(ScraperError::PrivateProfile);
        }

        let timeline = self.query_user(&client, username, Query::Posts).await?;
        let (posts, has_next_page) = parse_posts(&timeline)?;
        info!("Scraped {} posts for {} (more available: {})", posts.len(), username, has_next_page);

        user.reels = Some(posts.iter().filter(|post| post.is_video).map(reel_from_post).collect());
        user.posts = Some(posts);
        user.posts_limited = has_next_page;

        Ok(user)
    }

    async fn query_user(&self, client: &Client, username: &str, query: Query) -> Result<Value, ScraperError> {
        let mut doc_id = self.doc_id(query);
        let mut refreshed = false;

        loop {
            let body = self.post_graphql(client, &doc_id, query.variables(username)).await?;

            match classify_graphql_response(body) {
                GraphqlOutcome::User(user) => return Ok(user),
                GraphqlOutcome::NotFound => return Err(ScraperError::ProfileNotFound),
                GraphqlOutcome::StaleDocId if !refreshed => {
                    refreshed = true;
                    doc_id = self.replacement_doc_id(client, username, query, &doc_id).await?;
                }
                GraphqlOutcome::StaleDocId => {
                    return Err(ScraperError::UpstreamError(format!(
                        "{} rejected refreshed doc_id {}",
                        query.name(),
                        doc_id
                    )));
                }
                GraphqlOutcome::Rejected(summary) => {
                    return Err(ScraperError::UpstreamError(format!(
                        "{} request rejected: {}",
                        query.name(),
                        summary
                    )));
                }
            }
        }
    }

    /// Fetches the profile page at most once per cooldown, shared by concurrent scrapes
    async fn replacement_doc_id(
        &self,
        client: &Client,
        username: &str,
        query: Query,
        rejected: &str,
    ) -> Result<String, ScraperError> {
        let _guard = self.refresh_lock.lock().await;

        let recently_refreshed = self
            .doc_ids
            .read()
            .refreshed_at
            .map_or(false, |at| at.elapsed() < DOC_ID_REFRESH_COOLDOWN);

        if self.doc_id(query) == rejected && !recently_refreshed {
            warn!("{} rejected doc_id {}, refreshing doc_ids from profile page", query.name(), rejected);
            self.refresh_doc_ids(client, username).await?;
        }

        let latest = self.doc_id(query);
        if latest == rejected {
            return Err(ScraperError::UpstreamError(format!(
                "{} rejected doc_id {} and no other is known",
                query.name(),
                rejected
            )));
        }

        Ok(latest)
    }

    fn doc_id(&self, query: Query) -> String {
        let doc_ids = self.doc_ids.read();
        match query {
            Query::Profile => doc_ids.profile.clone(),
            Query::Posts => doc_ids.posts.clone(),
        }
    }

    async fn post_graphql(&self, client: &Client, doc_id: &str, variables: Value) -> Result<Value, ScraperError> {
        // Instagram accepts any well-formed lsd token as long as the header and form field match
        let lsd = random_token();
        let variables = variables.to_string();
        let form = [
            ("__a", "1"),
            ("lsd", lsd.as_str()),
            ("variables", variables.as_str()),
            ("doc_id", doc_id),
        ];

        let response = client
            .post(GRAPHQL_URL)
            .header("Accept", "*/*")
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Origin", "https://www.instagram.com")
            .header("Referer", "https://www.instagram.com/")
            .header("X-IG-App-ID", IG_APP_ID)
            .header("X-FB-LSD", &lsd)
            .header("Sec-Fetch-Site", "same-origin")
            .header("Sec-Fetch-Mode", "cors")
            .header("Sec-Fetch-Dest", "empty")
            .form(&form)
            .send()
            .await
            .map_err(|e| ScraperError::ProxyError(format!("Proxy request failed: {}", e)))?;

        let body = read_body(response).await?;

        // Some error payloads carry an anti-JSON-hijacking prefix
        serde_json::from_str(body.trim_start_matches("for (;;);")).map_err(|e| {
            ScraperError::ParsingError(format!("Invalid GraphQL JSON ({}): {}", e, preview(&body)))
        })
    }

    async fn refresh_doc_ids(&self, client: &Client, username: &str) -> Result<(), ScraperError> {
        // Without navigation headers Instagram serves an error page that lacks the query ids
        let response = client
            .get(format!("https://www.instagram.com/{}/", username))
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "none")
            .header("Sec-Fetch-User", "?1")
            .header("Upgrade-Insecure-Requests", "1")
            .send()
            .await
            .map_err(|e| ScraperError::ProxyError(format!("Proxy request failed: {}", e)))?;

        let html = read_body(response).await?;
        self.doc_ids.write().refreshed_at = Some(Instant::now());
        let (profile, posts) = extract_doc_ids(&html);

        if profile.is_none() && posts.is_none() {
            return Err(ScraperError::ParsingError(
                "No GraphQL doc_ids found in profile page".to_string(),
            ));
        }

        let mut doc_ids = self.doc_ids.write();
        if let Some(id) = profile {
            doc_ids.profile = id;
        }
        if let Some(id) = posts {
            doc_ids.posts = id;
        }
        info!("Using GraphQL doc_ids: profile={}, posts={}", doc_ids.profile, doc_ids.posts);

        Ok(())
    }
}

async fn read_body(response: Response) -> Result<String, ScraperError> {
    let status = response.status();

    if status.is_redirection() {
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        return Err(ScraperError::UpstreamError(format!("Redirected to {}", location)));
    }

    if status == StatusCode::TOO_MANY_REQUESTS {
        return Err(ScraperError::RateLimited);
    }

    let body = response.text().await?;

    match status {
        s if s.is_success() => Ok(body),
        StatusCode::NOT_FOUND => Err(ScraperError::ProfileNotFound),
        s => Err(ScraperError::UpstreamError(format!("HTTP {}: {}", s, preview(&body)))),
    }
}

fn classify_graphql_response(body: Value) -> GraphqlOutcome {
    match body.pointer("/data/xig_user_by_username") {
        Some(Value::Null) => GraphqlOutcome::NotFound,
        Some(user) => GraphqlOutcome::User(user.clone()),
        // Unknown or retired doc_ids return GraphQL errors without data
        None if body.get("errors").is_some() => GraphqlOutcome::StaleDocId,
        None => GraphqlOutcome::Rejected(preview(&body.to_string())),
    }
}

fn extract_doc_ids(html: &str) -> (Option<String>, Option<String>) {
    let re = Regex::new(
        r#""queryID":"(\d+)","variables":\{[^}]*\},"queryName":"(PolarisLoggedOutDesktopWWW\w+Query)""#,
    )
    .expect("valid doc_id regex");

    let mut profile = None;
    let mut posts = None;

    for caps in re.captures_iter(html) {
        match &caps[2] {
            PROFILE_QUERY => profile = Some(caps[1].to_string()),
            POSTS_QUERY => posts = Some(caps[1].to_string()),
            _ => {}
        }
    }

    (profile, posts)
}

fn parse_profile(user: &Value, username: &str) -> InstagramUser {
    let text = |key: &str| user.get(key).and_then(Value::as_str).map(str::to_string);
    let flag = |key: &str| user.get(key).and_then(Value::as_bool).unwrap_or(false);
    let count = |key: &str| user.get(key).and_then(Value::as_u64);

    InstagramUser {
        username: username.to_string(),
        full_name: text("full_name"),
        biography: text("biography"),
        profile_pic_url: text("profile_pic_url"),
        is_private: flag("is_private"),
        is_verified: flag("is_verified"),
        external_url: user
            .pointer("/bio_links/0/url")
            .and_then(Value::as_str)
            .map(str::to_string),
        stats: InstagramUserStats {
            // null for logged-out requests
            posts_count: count("all_media_count"),
            followers_count: count("follower_count"),
            following_count: count("following_count"),
        },
        posts: None,
        reels: None,
        scraped_at: Utc::now(),
        posts_limited: false,
    }
}

// Errors instead of returning an empty list that would replace cached posts
fn parse_posts(user: &Value) -> Result<(Vec<InstagramPost>, bool), ScraperError> {
    let edges = user
        .pointer("/polaris_ordered_timeline_connection/edges")
        .and_then(Value::as_array)
        .ok_or_else(|| ScraperError::ParsingError("Posts response has no timeline edges".to_string()))?;

    let posts: Vec<InstagramPost> = edges
        .iter()
        .filter_map(|edge| edge.get("node"))
        .filter_map(parse_post)
        .collect();

    if posts.is_empty() && !edges.is_empty() {
        return Err(ScraperError::ParsingError(format!(
            "None of the {} timeline posts could be parsed",
            edges.len()
        )));
    }

    let has_next_page = user
        .pointer("/polaris_ordered_timeline_connection/page_info/has_next_page")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok((posts, has_next_page))
}

// Logged-out responses carry no like/comment/view counts nor video URLs
fn parse_post(node: &Value) -> Option<InstagramPost> {
    let pk = node.get("pk")?.as_str()?;
    let display_url = node.get("display_uri")?.as_str()?.to_string();

    Some(InstagramPost {
        id: pk.to_string(),
        shortcode: node.get("code")?.as_str()?.to_string(),
        thumbnail_url: Some(display_url.clone()),
        display_url,
        caption: node
            .pointer("/caption/text")
            .and_then(Value::as_str)
            .map(str::to_string),
        likes_count: None,
        comments_count: None,
        timestamp: timestamp_from_media_id(pk),
        is_video: node.get("media_type").and_then(Value::as_u64) == Some(MEDIA_TYPE_VIDEO),
        video_url: None,
        video_view_count: None,
    })
}

fn reel_from_post(post: &InstagramPost) -> InstagramReel {
    InstagramReel {
        id: post.id.clone(),
        shortcode: post.shortcode.clone(),
        display_url: post.display_url.clone(),
        video_url: post.video_url.clone(),
        caption: post.caption.clone(),
        views_count: post.video_view_count,
        likes_count: post.likes_count,
        comments_count: post.comments_count,
        timestamp: post.timestamp,
    }
}

fn timestamp_from_media_id(media_id: &str) -> Option<DateTime<Utc>> {
    let id: u64 = media_id.parse().ok()?;
    let millis = i64::try_from(id >> MEDIA_ID_TIMESTAMP_SHIFT).ok()? + INSTAGRAM_EPOCH_MS;
    // Whole seconds, like the taken_at timestamps Instagram used to return
    Utc.timestamp_opt(millis / 1000, 0).single()
}

fn random_token() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(11)
        .map(char::from)
        .collect()
}

fn preview(text: &str) -> String {
    text.chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_graphql_responses() {
        let user = json!({ "pk": "1", "username": "someone" });
        assert_eq!(
            classify_graphql_response(json!({ "data": { "xig_user_by_username": user.clone() } })),
            GraphqlOutcome::User(user)
        );
        assert_eq!(
            classify_graphql_response(json!({
                "data": { "xig_user_by_username": null },
                "errors": [{ "message": "A server error field_exception occured." }]
            })),
            GraphqlOutcome::NotFound
        );
        assert_eq!(
            classify_graphql_response(json!({
                "errors": [{ "message": "Exception was thrown during the execution of a query." }]
            })),
            GraphqlOutcome::StaleDocId
        );
        assert!(matches!(
            classify_graphql_response(json!({ "error": 1357054, "errorSummary": "Your Request Couldn't be Processed" })),
            GraphqlOutcome::Rejected(_)
        ));
    }

    #[test]
    fn decodes_timestamp_from_media_id() {
        let timestamp = timestamp_from_media_id("3929626071510717909").unwrap();
        assert_eq!(timestamp.to_rfc3339(), "2026-06-28T17:32:33+00:00");
        assert_eq!(timestamp_from_media_id("not-a-number"), None);
    }

    #[test]
    fn parses_posts_and_reels() {
        let timeline = json!({
            "polaris_ordered_timeline_connection": {
                "edges": [
                    { "node": {
                        "pk": "3929626071510717909",
                        "code": "DaI1heMgpXV",
                        "display_uri": "https://scontent.cdninstagram.com/v/image_n.jpg",
                        "caption": { "text": "Objectif atteint" },
                        "media_type": 1,
                        "product_type": "feed"
                    } },
                    { "node": {
                        "pk": "3929626071510717910",
                        "code": "Ddl_PI8A7dv",
                        "display_uri": "https://scontent.cdninstagram.com/v/video_n.jpg",
                        "caption": null,
                        "media_type": 2,
                        "product_type": "clips"
                    } },
                    { "node": { "code": "missing-pk" } }
                ],
                "page_info": { "has_next_page": true, "end_cursor": "cursor" }
            }
        });

        let (posts, has_next_page) = parse_posts(&timeline).unwrap();
        assert!(has_next_page);
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].id, "3929626071510717909");
        assert_eq!(posts[0].shortcode, "DaI1heMgpXV");
        assert_eq!(posts[0].caption.as_deref(), Some("Objectif atteint"));
        assert!(!posts[0].is_video);
        assert!(posts[1].is_video);
        assert_eq!(posts[1].caption, None);

        let reels: Vec<InstagramReel> = posts.iter().filter(|p| p.is_video).map(reel_from_post).collect();
        assert_eq!(reels.len(), 1);
        assert_eq!(reels[0].shortcode, "Ddl_PI8A7dv");
    }

    #[test]
    fn rejects_timelines_that_would_empty_the_cache() {
        assert!(parse_posts(&json!({ "polaris_ordered_timeline_connection": null })).is_err());
        assert!(parse_posts(&json!({
            "polaris_ordered_timeline_connection": { "edges": [{ "node": { "code": "no-pk" } }] }
        }))
        .is_err());

        let (posts, has_next_page) = parse_posts(&json!({
            "polaris_ordered_timeline_connection": { "edges": [], "page_info": { "has_next_page": false } }
        }))
        .unwrap();
        assert!(posts.is_empty());
        assert!(!has_next_page);
    }

    #[test]
    fn parses_profile() {
        let profile = json!({
            "username": "mauguio_basket",
            "full_name": "Mauguio Carnon Basket",
            "biography": "Club de Basket",
            "profile_pic_url": "https://scontent.cdninstagram.com/v/pic_n.jpg",
            "is_private": false,
            "is_verified": false,
            "bio_links": [{ "url": "https://linktr.ee/mauguiobasket" }],
            "follower_count": 1456,
            "following_count": 141,
            "all_media_count": null
        });

        let user = parse_profile(&profile, "mauguio_basket");
        assert_eq!(user.full_name.as_deref(), Some("Mauguio Carnon Basket"));
        assert_eq!(user.external_url.as_deref(), Some("https://linktr.ee/mauguiobasket"));
        assert_eq!(user.stats.followers_count, Some(1456));
        assert_eq!(user.stats.following_count, Some(141));
        assert_eq!(user.stats.posts_count, None);
        assert!(!user.is_private);
    }

    #[test]
    fn extracts_doc_ids_from_profile_page() {
        let html = r#"{"actorID":"0","preloaderID":"adp_a","queryID":"27981003384861049","variables":{"username":"u"},"queryName":"PolarisLoggedOutDesktopWWWProfileRootContentQuery"},{"actorID":"0","preloaderID":"adp_b","queryID":"27553725110923321","variables":{"first":12,"username":"u"},"queryName":"PolarisLoggedOutDesktopWWWProfilePostsTabContentQuery"}"#;

        assert_eq!(
            extract_doc_ids(html),
            (Some("27981003384861049".to_string()), Some("27553725110923321".to_string()))
        );
        assert_eq!(extract_doc_ids("<html>error page</html>"), (None, None));
    }
}
