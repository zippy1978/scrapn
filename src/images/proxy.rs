use std::time::Duration;
use crate::images::tools::ImageProxyError;
use reqwest::Client;

pub struct ImageProxy {
    timeout: Duration,
    client: Client,
}

impl ImageProxy {
    pub fn new(timeout: u64) -> Self {
        let timeout = Duration::from_secs(timeout);
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .pool_max_idle_per_host(100)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Some(Duration::from_secs(60)))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self { timeout, client }
    }

    
    // Fetch an image from a URL
    pub async fn fetch_image(&self, url: &str) -> Result<(Vec<u8>, String), ImageProxyError> {
        self.make_request(url, None).await
    }

    // Make actual HTTP request with or without proxy
    async fn make_request(&self, url: &str, proxy_url: Option<&str>) -> Result<(Vec<u8>, String), ImageProxyError> {
        // Use the shared client unless a proxy is required (proxies are client-wide in reqwest)
        let client = if let Some(proxy) = proxy_url {
            let builder = reqwest::Client::builder()
                .timeout(self.timeout)
                .pool_max_idle_per_host(100)
                .pool_idle_timeout(Duration::from_secs(90))
                .tcp_keepalive(Some(Duration::from_secs(60)));
            let builder = match reqwest::Proxy::all(proxy) {
                Ok(proxy) => builder.proxy(proxy),
                Err(e) => return Err(ImageProxyError::ProxyError(format!("Failed to create proxy: {}", e))),
            };
            match builder.build() {
                Ok(c) => c,
                Err(e) => return Err(ImageProxyError::ProxyError(format!("Failed to build client: {}", e))),
            }
        } else {
            self.client.clone()
        };
        
        // Build request with headers matching browser request
        let request = client.get(url)
            .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.4 Safari/605.1.15")
            .header("Accept", "image/avif,image/webp,image/apng,image/*,*/*;q=0.8")
            .header("Accept-Language", "fr-FR,fr;q=0.9")
            .header("Accept-Encoding", "gzip, deflate, br");
        
        match request.send().await {
            Ok(response) => {
                let status = response.status();
                if !status.is_success() {
                    log::error!("Image request failed with status: {}", status);
                    return Err(ImageProxyError::ImageError(
                        format!("Image request failed with status: {}", status)
                    ));
                }

                log::info!("Image request successful");
                
                // Get the content-type from headers or default to octet-stream
                let content_type = response.headers()
                    .get("content-type")
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("application/octet-stream")
                    .to_string();
                
                match response.bytes().await {
                    Ok(bytes) => {
                        let image_data = bytes.to_vec();
                        
                        // If content type is missing or generic, try to detect from image data
                        let content_type = if content_type == "application/octet-stream" || content_type.is_empty() {
                            self.detect_image_type(&image_data)
                        } else {
                            content_type
                        };
                        
                        Ok((image_data, content_type))
                    },
                    Err(e) => Err(ImageProxyError::NetworkError(e)),
                }
            },
            Err(e) => Err(ImageProxyError::NetworkError(e)),
        }
    }

    // Function to detect image type from the image data
    fn detect_image_type(&self, data: &[u8]) -> String {
        // Check for common image file signatures
        if data.len() >= 4 {
            if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
                return "image/jpeg".to_string();
            } else if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
                return "image/png".to_string();
            } else if data.starts_with(&[0x47, 0x49, 0x46]) {
                return "image/gif".to_string();
            } else if data.starts_with(&[0x52, 0x49, 0x46, 0x46]) && data.len() >= 12 {
                // Check for WEBP signature
                if data[8..12] == [0x57, 0x45, 0x42, 0x50] {
                    return "image/webp".to_string();
                }
            }
        }
        
        // Default to JPEG if we can't detect
        "image/jpeg".to_string()
    }
} 