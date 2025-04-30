use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub port: u16,
    pub address: String,
    pub instagram_cache_duration: u64,
    pub timeout: u64,
    pub user_agent: String,
    pub instagram_username_whitelist: Option<Vec<String>>,
    pub instagram_cookies: Option<String>,
}
 