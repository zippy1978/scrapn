use std::collections::HashMap;
use std::time::{Duration, Instant};
use parking_lot::RwLock;
use crate::models::instagram::{InstagramUser, InstagramPost, InstagramReel};

#[derive(Debug, Clone)]
pub struct CacheEntry<T> {
    pub data: T,
    pub inserted_at: Instant,
    pub expires_at: Instant,
}

impl<T: Clone> CacheEntry<T> {
    pub fn new(data: T, ttl: Duration) -> Self {
        let now = Instant::now();
        Self {
            data,
            inserted_at: now,
            expires_at: now + ttl,
        }
    }

    pub fn is_expired(&self) -> bool {
        Instant::now() > self.expires_at
    }

    pub fn age(&self) -> Duration {
        Instant::now().saturating_duration_since(self.inserted_at)
    }
}

pub struct InstagramCache {
    users: RwLock<HashMap<String, CacheEntry<InstagramUser>>>,
    pub cache_duration: Duration,
}

impl InstagramCache {
    pub fn new(cache_days: u64) -> Self {
        Self {
            users: RwLock::new(HashMap::new()),
            cache_duration: Duration::from_secs(cache_days * 24 * 60 * 60),
        }
    }

    pub fn get_user(&self, username: &str) -> Option<(InstagramUser, u64)> {
        let users = self.users.read();
        
        if let Some(entry) = users.get(username) {
            if !entry.is_expired() {
                return Some((entry.data.clone(), entry.age().as_secs()));
            }
        }
        
        None
    }

    pub fn get_user_even_expired(&self, username: &str) -> Option<(InstagramUser, u64)> {
        let users = self.users.read();
        
        if let Some(entry) = users.get(username) {
            return Some((entry.data.clone(), entry.age().as_secs()));
        }
        
        None
    }

    pub fn store_user(&self, user: InstagramUser) {
        let mut users = self.users.write();
        users.insert(
            user.username.clone(),
            CacheEntry::new(user, self.cache_duration),
        );
    }

    pub fn get_posts(&self, username: &str) -> Option<(Vec<InstagramPost>, u64)> {
        let (user, age) = self.get_user(username)?;
        
        user.posts.map(|posts| (posts, age))
    }

    pub fn get_posts_even_expired(&self, username: &str) -> Option<(Vec<InstagramPost>, u64)> {
        let (user, age) = self.get_user_even_expired(username)?;
        
        user.posts.map(|posts| (posts, age))
    }

    pub fn get_reels(&self, username: &str) -> Option<(Vec<InstagramReel>, u64)> {
        let (user, age) = self.get_user(username)?;
        
        user.reels.map(|reels| (reels, age))
    }

    pub fn get_reels_even_expired(&self, username: &str) -> Option<(Vec<InstagramReel>, u64)> {
        let (user, age) = self.get_user_even_expired(username)?;
        
        user.reels.map(|reels| (reels, age))
    }
} 