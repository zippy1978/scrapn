use std::collections::HashMap;
use std::time::{Duration, Instant};
use std::sync::{Arc, Mutex};
use log::{info, warn};

#[derive(Debug, Clone, PartialEq)]
pub enum ProxyProtocol {
    HTTP,
    HTTPS,
    SOCKS5,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct ProxyManager {
    proxies: Arc<Mutex<HashMap<String, ProxyStatus>>>,
    unavailable_duration: Duration,
}

#[derive(Debug, Clone)]
pub struct ProxyStatus {
    pub available: bool,
    pub last_failure: Option<Instant>,
    pub protocol: ProxyProtocol,
}

impl ProxyManager {
    pub fn new(proxy_list: Option<Vec<String>>, unavailable_duration_hours: u64) -> Self {
        let mut proxies = HashMap::new();
        
        if let Some(list) = proxy_list {
            for proxy in list {
                let protocol = Self::detect_proxy_protocol(&proxy);
                proxies.insert(proxy, ProxyStatus { 
                    available: true, 
                    last_failure: None,
                    protocol,
                });
            }
        }
        
        let manager = ProxyManager {
            proxies: Arc::new(Mutex::new(proxies)),
            unavailable_duration: Duration::from_secs(unavailable_duration_hours * 3600),
        };
        
        // Log the detected protocols
        manager.debug_proxy_list();
        
        manager
    }
    
    pub fn get_random_proxy(&self) -> Option<String> {
        let mut proxies_guard = self.proxies.lock().unwrap();
        
        // Check if any unavailable proxies should be marked available again
        for (_, status) in proxies_guard.iter_mut() {
            if !status.available {
                if let Some(failure_time) = status.last_failure {
                    if failure_time.elapsed() >= self.unavailable_duration {
                        status.available = true;
                        status.last_failure = None;
                    }
                }
            }
        }
        
        // Get all available proxies
        let available_proxies: Vec<String> = proxies_guard
            .iter()
            .filter(|(_, status)| status.available)
            .map(|(proxy, _)| proxy.clone())
            .collect();
        
        if available_proxies.is_empty() {
            return None;
        }
        
        // Select a random proxy
        use rand::seq::SliceRandom;
        available_proxies.choose(&mut rand::thread_rng()).cloned()
    }
    
    pub fn mark_proxy_unavailable(&self, proxy: &str) {
        if let Some(status) = self.proxies.lock().unwrap().get_mut(proxy) {
            status.available = false;
            status.last_failure = Some(Instant::now());
        }
    }
    
    pub fn get_proxy_count(&self) -> (usize, usize) {
        let proxies_guard = self.proxies.lock().unwrap();
        let total = proxies_guard.len();
        let available = proxies_guard.values().filter(|status| status.available).count();
        (available, total)
    }
    
    pub fn get_proxy_protocol(&self, proxy: &str) -> ProxyProtocol {
        let proxies_guard = self.proxies.lock().unwrap();
        match proxies_guard.get(proxy) {
            Some(status) => status.protocol.clone(),
            None => ProxyProtocol::Unknown,
        }
    }
    
    /// Detect proxy protocol from URL string
    fn detect_proxy_protocol(proxy_url: &str) -> ProxyProtocol {
        if proxy_url.starts_with("http://") {
            ProxyProtocol::HTTP
        } else if proxy_url.starts_with("https://") {
            ProxyProtocol::HTTPS
        } else if proxy_url.starts_with("socks5://") || proxy_url.starts_with("socks://") {
            ProxyProtocol::SOCKS5
        } else {
            // Try to guess based on port number or return unknown
            if proxy_url.contains(":1080") || proxy_url.contains(":9050") {
                // Common SOCKS ports
                // 1080 is typical for SOCKS
                // 9050 is used by Tor (SOCKS)
                ProxyProtocol::SOCKS5
            } else if proxy_url.contains(":8080") || proxy_url.contains(":3128") || proxy_url.contains(":80") {
                // Common HTTP proxy ports
                ProxyProtocol::HTTP
            } else if proxy_url.contains(":443") {
                // Common HTTPS port
                ProxyProtocol::HTTPS
            } else {
                // Default to HTTP for unknown
                ProxyProtocol::HTTP
            }
        }
    }
    
    /// Normalize proxy URL to ensure it has the correct protocol prefix
    pub fn normalize_proxy_url(&self, proxy_url: &str) -> String {
        // If the URL already has a protocol, return it unchanged
        if proxy_url.starts_with("http://") || 
           proxy_url.starts_with("https://") || 
           proxy_url.starts_with("socks5://") || 
           proxy_url.starts_with("socks://") {
            return proxy_url.to_string();
        }
        
        // Get the protocol from our stored data if available
        let protocol = self.get_proxy_protocol(proxy_url);
        
        // Prepend the appropriate protocol
        match protocol {
            ProxyProtocol::HTTP => format!("http://{}", proxy_url),
            ProxyProtocol::HTTPS => format!("https://{}", proxy_url),
            ProxyProtocol::SOCKS5 => format!("socks5://{}", proxy_url),
            ProxyProtocol::Unknown => {
                // Default to HTTP if unknown
                format!("http://{}", proxy_url)
            }
        }
    }
    
    /// Print debug information about all proxies
    pub fn debug_proxy_list(&self) {
        let proxies_guard = self.proxies.lock().unwrap();
        if proxies_guard.is_empty() {
            info!("No proxies configured");
            return;
        }
        
        info!("Proxy configuration:");
        info!("Supported protocols: HTTP, HTTPS, SOCKS5");
        
        for (url, status) in proxies_guard.iter() {
            let normalized = if url.starts_with("http://") || 
                              url.starts_with("https://") || 
                              url.starts_with("socks5://") || 
                              url.starts_with("socks://") {
                url.to_string()
            } else {
                match status.protocol {
                    ProxyProtocol::HTTP => format!("http://{}", url),
                    ProxyProtocol::HTTPS => format!("https://{}", url),
                    ProxyProtocol::SOCKS5 => format!("socks5://{}", url),
                    ProxyProtocol::Unknown => format!("http://{}", url),
                }
            };
            
            info!("  Proxy: {} (Detected protocol: {:?}, Normalized: {})", 
                 url, status.protocol, normalized);
            
            if !url.starts_with("http://") && 
               !url.starts_with("https://") && 
               !url.starts_with("socks5://") && 
               !url.starts_with("socks://") {
                warn!("  Warning: Proxy URL {} doesn't include protocol. Will use detected protocol: {:?}", 
                     url, status.protocol);
            }
        }
    }
    
    /// Reset all proxies to available state (used for retries)
    pub fn reset_all_proxies(&self) {
        let mut proxies_guard = self.proxies.lock().unwrap();
        for (_, status) in proxies_guard.iter_mut() {
            status.available = true;
            status.last_failure = None;
        }
        info!("Reset all proxies to available state for retry");
    }
} 