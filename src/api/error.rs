use crate::images::ImageProxyError;
use crate::scrapers::instagram::ScraperError;
use rocket::http::Status;
use serde_json::json;

#[derive(Debug)]
pub enum ApiError {
    ScraperError(ScraperError),
    ImageError(ImageProxyError),
}

impl From<ScraperError> for ApiError {
    fn from(error: ScraperError) -> Self {
        ApiError::ScraperError(error)
    }
}

impl From<ImageProxyError> for ApiError {
    fn from(error: ImageProxyError) -> Self {
        ApiError::ImageError(error)
    }
}

impl<'r> rocket::response::Responder<'r, 'static> for ApiError {
    fn respond_to(self, _: &'r rocket::Request<'_>) -> rocket::response::Result<'static> {
        match self {
            ApiError::ScraperError(ScraperError::ProfileNotFound) => rocket::Response::build()
                .status(Status::NotFound)
                .sized_body(None, std::io::Cursor::new("Profile not found"))
                .ok(),
            ApiError::ScraperError(ScraperError::PrivateProfile) => {
                let body = json!({
                    "error": "Profile is private",
                    "message": "The requested profile is private and cannot be accessed"
                })
                .to_string();

                rocket::Response::build()
                    .status(Status::Forbidden)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            }
            ApiError::ScraperError(ScraperError::RateLimited) => {
                let body = json!({
                    "error": "Rate limited",
                    "message": "Too many requests, please try again later"
                })
                .to_string();

                rocket::Response::build()
                    .status(Status::TooManyRequests)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            }
            ApiError::ScraperError(ScraperError::UnauthorizedAccess(message)) => {
                let body = json!({
                    "error": "Unauthorized",
                    "message": message
                })
                .to_string();

                rocket::Response::build()
                    .status(Status::Unauthorized)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            }
            ApiError::ScraperError(ScraperError::ProxyError(error)) => {
                let body = json!({
                    "error": "Proxy error",
                    "message": error
                })
                .to_string();

                rocket::Response::build()
                    .status(Status::BadGateway)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            }
            ApiError::ScraperError(ScraperError::AllProxiesFailed) => {
                let body = json!({
                    "error": "All proxies failed",
                    "message": "All configured proxies failed to connect"
                })
                .to_string();

                rocket::Response::build()
                    .status(Status::ServiceUnavailable)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            }
            ApiError::ScraperError(ScraperError::NetworkError(error)) => {
                let body = json!({
                    "error": "Network error",
                    "message": error.to_string()
                })
                .to_string();

                rocket::Response::build()
                    .status(Status::ServiceUnavailable)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            }
            ApiError::ScraperError(ScraperError::ParsingError(e)) => rocket::Response::build()
                .status(Status::InternalServerError)
                .sized_body(
                    None,
                    std::io::Cursor::new(format!("Error parsing Instagram page: {}", e)),
                )
                .ok(),
            ApiError::ImageError(ImageProxyError::NetworkError(error)) => {
                let body = json!({
                    "error": "Image network error",
                    "message": error.to_string()
                })
                .to_string();

                rocket::Response::build()
                    .status(Status::ServiceUnavailable)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            }
            ApiError::ImageError(ImageProxyError::ProxyError(error)) => {
                let body = json!({
                    "error": "Image proxy error",
                    "message": error
                })
                .to_string();

                rocket::Response::build()
                    .status(Status::BadGateway)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            }
            ApiError::ImageError(ImageProxyError::ImageError(error)) => {
                let body = json!({
                    "error": "Image processing error",
                    "message": error
                })
                .to_string();

                rocket::Response::build()
                    .status(Status::InternalServerError)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            }
            ApiError::ImageError(ImageProxyError::ConversionError(error)) => {
                let body = json!({
                    "error": "Image conversion error",
                    "message": error
                })
                .to_string();

                rocket::Response::build()
                    .status(Status::BadRequest)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            }
           
        }
    }
}
