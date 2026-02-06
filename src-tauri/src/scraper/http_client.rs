use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use texting_robots::{get_robots_url, Robot};
use tracing::{info, warn};
use url::Url;

/// Per-domain rate limiter using a simple last-request-time tracking approach.
/// Ensures at most `requests_per_second` requests per domain.
pub struct RateLimiter {
    last_request: Mutex<HashMap<String, Instant>>,
    min_interval: Duration,
}

impl RateLimiter {
    pub fn new(requests_per_second: f64) -> Self {
        Self {
            last_request: Mutex::new(HashMap::new()),
            min_interval: Duration::from_secs_f64(1.0 / requests_per_second),
        }
    }

    /// Wait until enough time has elapsed since the last request to the same domain.
    /// If `extra_delay` is provided and greater than `min_interval`, use it instead
    /// (this supports crawl-delay from robots.txt).
    pub async fn wait_for_domain(&self, url: &str, extra_delay: Option<Duration>) -> Result<(), String> {
        let domain = Url::parse(url)
            .map_err(|e| format!("Failed to parse URL '{}': {}", url, e))?
            .host_str()
            .ok_or_else(|| format!("No host in URL: {}", url))?
            .to_string();

        let effective_interval = match extra_delay {
            Some(delay) if delay > self.min_interval => delay,
            _ => self.min_interval,
        };

        let sleep_duration = {
            let map = self.last_request.lock().unwrap();
            if let Some(last) = map.get(&domain) {
                let elapsed = last.elapsed();
                if elapsed < effective_interval {
                    Some(effective_interval - elapsed)
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(duration) = sleep_duration {
            tokio::time::sleep(duration).await;
        }

        // Update last request time after waiting
        let mut map = self.last_request.lock().unwrap();
        map.insert(domain, Instant::now());
        Ok(())
    }

    /// Get the default minimum interval between requests.
    pub fn min_interval(&self) -> Duration {
        self.min_interval
    }
}

/// Cached robots.txt entries with a 1-hour TTL.
/// Stores parsed Robot objects per domain to avoid re-fetching on every request.
pub struct RobotsCache {
    cache: Mutex<HashMap<String, (Robot, Option<Duration>, Instant)>>,
    ttl: Duration,
}

impl RobotsCache {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            ttl: Duration::from_secs(3600), // 1 hour TTL
        }
    }

    /// Check if the given URL is allowed by the domain's robots.txt.
    /// Returns (is_allowed, crawl_delay) where crawl_delay is the
    /// Crawl-delay directive if present.
    /// Fetches and caches robots.txt if not already cached.
    /// Returns Ok(true) if robots.txt is missing (404) -- all allowed.
    pub async fn check(
        &self,
        client: &reqwest::Client,
        url: &str,
    ) -> Result<(bool, Option<Duration>), String> {
        let robots_url = get_robots_url(url)
            .map_err(|e| format!("Failed to get robots.txt URL for '{}': {}", url, e))?;

        let domain = Url::parse(url)
            .map_err(|e| format!("Failed to parse URL: {}", e))?
            .host_str()
            .ok_or_else(|| format!("No host in URL: {}", url))?
            .to_string();

        // Check cache first
        {
            let cache = self.cache.lock().unwrap();
            if let Some((robot, crawl_delay, cached_at)) = cache.get(&domain) {
                if cached_at.elapsed() < self.ttl {
                    let allowed = robot.allowed(url);
                    return Ok((allowed, *crawl_delay));
                }
            }
        }

        // Fetch robots.txt
        info!("Fetching robots.txt for domain: {}", domain);
        let response = client
            .get(&robots_url)
            .header("User-Agent", "BambuMate/1.0")
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("Failed to fetch robots.txt for '{}': {}", domain, e))?;

        let status = response.status();

        // Handle 404: no robots.txt means all allowed
        if status == reqwest::StatusCode::NOT_FOUND {
            info!("No robots.txt found for {} (404), all URLs allowed", domain);
            // Create a permissive robot (empty robots.txt allows everything)
            let robot = Robot::new("BambuMate/1.0", b"")
                .map_err(|e| format!("Failed to create default robot: {}", e))?;
            let mut cache = self.cache.lock().unwrap();
            cache.insert(domain, (robot, None, Instant::now()));
            return Ok((true, None));
        }

        if !status.is_success() {
            warn!("robots.txt fetch returned {} for {}, assuming all allowed", status, domain);
            let robot = Robot::new("BambuMate/1.0", b"")
                .map_err(|e| format!("Failed to create default robot: {}", e))?;
            let mut cache = self.cache.lock().unwrap();
            cache.insert(domain, (robot, None, Instant::now()));
            return Ok((true, None));
        }

        let body = response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read robots.txt body: {}", e))?;

        let robot = Robot::new("BambuMate/1.0", &body)
            .map_err(|e| format!("Failed to parse robots.txt for '{}': {}", domain, e))?;

        let crawl_delay = robot.delay.map(|d| Duration::from_secs_f32(d));
        let allowed = robot.allowed(url);

        // Cache the result
        let mut cache = self.cache.lock().unwrap();
        cache.insert(domain, (robot, crawl_delay, Instant::now()));

        Ok((allowed, crawl_delay))
    }

    /// Check a URL against a pre-built Robot (for testing without HTTP).
    pub fn check_against_robot(robot: &Robot, url: &str) -> bool {
        robot.allowed(url)
    }
}

/// Rate-limited HTTP client with robots.txt checking.
/// Ensures polite scraping behavior: checks robots.txt before every fetch,
/// rate-limits to 1 request/second per domain (or the crawl-delay, whichever is higher).
pub struct ScraperHttpClient {
    client: reqwest::Client,
    rate_limiter: RateLimiter,
    robots_cache: RobotsCache,
}

impl ScraperHttpClient {
    /// Create a new ScraperHttpClient with default settings:
    /// - 1 request/second per domain
    /// - User-Agent: BambuMate/1.0
    /// - 30 second page fetch timeout
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent("BambuMate/1.0")
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build reqwest client");

        Self {
            client,
            rate_limiter: RateLimiter::new(1.0),
            robots_cache: RobotsCache::new(),
        }
    }

    /// Fetch a page's HTML content.
    /// 1. Checks robots.txt -- returns Err if URL is disallowed
    /// 2. Waits for rate limit (using max of default interval and crawl-delay)
    /// 3. Fetches the page and returns HTML body
    pub async fn fetch_page(&self, url: &str) -> Result<String, String> {
        // Step 1: Check robots.txt
        let (allowed, crawl_delay) = self.robots_cache.check(&self.client, url).await?;
        if !allowed {
            return Err(format!("URL blocked by robots.txt: {}", url));
        }

        // Step 2: Rate limit (use crawl-delay if higher than default)
        self.rate_limiter.wait_for_domain(url, crawl_delay).await?;

        // Step 3: Fetch the page
        info!("Fetching page: {}", url);
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch '{}': {}", url, e))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(format!(
                "HTTP error fetching '{}': {} {}",
                url,
                status.as_u16(),
                status.canonical_reason().unwrap_or("Unknown")
            ));
        }

        response
            .text()
            .await
            .map_err(|e| format!("Failed to read response body from '{}': {}", url, e))
    }

    /// Convert HTML to plain text for LLM consumption.
    /// Reduces token cost by stripping tags, scripts, styles, etc.
    /// Uses html2text with 120-character line width.
    pub fn html_to_text(html: &str) -> String {
        html2text::from_read(html.as_bytes(), 120).unwrap_or_else(|e| {
            warn!("html2text conversion failed: {}, returning raw text", e);
            html.to_string()
        })
    }

    /// Get a reference to the inner reqwest client (for use in extraction).
    pub fn inner_client(&self) -> &reqwest::Client {
        &self.client
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_to_text_strips_tags() {
        let html = "<h1>Hello</h1><p>World</p>";
        let text = ScraperHttpClient::html_to_text(html);
        assert!(text.contains("Hello"), "Expected 'Hello' in output: {}", text);
        assert!(text.contains("World"), "Expected 'World' in output: {}", text);
        assert!(!text.contains("<h1>"), "Expected no HTML tags in output: {}", text);
        assert!(!text.contains("<p>"), "Expected no HTML tags in output: {}", text);
    }

    #[test]
    fn test_html_to_text_complex_html() {
        let html = r#"
            <html>
            <head><title>Test</title></head>
            <body>
                <div class="nav">Navigation</div>
                <h1>Product Name</h1>
                <p>Nozzle Temperature: 190-220C</p>
                <table>
                    <tr><td>Bed Temp</td><td>50-60C</td></tr>
                </table>
            </body>
            </html>
        "#;
        let text = ScraperHttpClient::html_to_text(html);
        assert!(text.contains("Product Name"));
        assert!(text.contains("190-220"));
        assert!(text.contains("50-60"));
    }

    #[test]
    fn test_robots_txt_disallow() {
        let robots_txt = b"User-agent: *\nDisallow: /private/\nDisallow: /admin/";
        let robot = Robot::new("BambuMate/1.0", robots_txt).unwrap();

        assert!(!RobotsCache::check_against_robot(
            &robot,
            "https://example.com/private/page"
        ));
        assert!(!RobotsCache::check_against_robot(
            &robot,
            "https://example.com/admin/dashboard"
        ));
        assert!(RobotsCache::check_against_robot(
            &robot,
            "https://example.com/public/page"
        ));
        assert!(RobotsCache::check_against_robot(
            &robot,
            "https://example.com/products/pla"
        ));
    }

    #[test]
    fn test_robots_txt_crawl_delay() {
        let robots_txt = b"User-agent: *\nCrawl-delay: 5\nDisallow: /blocked/";
        let robot = Robot::new("BambuMate/1.0", robots_txt).unwrap();

        // Verify crawl delay is extracted
        assert_eq!(robot.delay, Some(5.0));

        // Verify the delay converted to Duration would be 5 seconds
        let delay = robot.delay.map(|d| Duration::from_secs_f32(d));
        assert_eq!(delay, Some(Duration::from_secs(5)));

        // The crawl delay should be higher than the default 1-second interval
        let default_interval = Duration::from_secs_f64(1.0);
        assert!(delay.unwrap() > default_interval);
    }

    #[test]
    fn test_robots_txt_wildcard_disallow() {
        // Test that wildcard user-agent disallow rules are respected
        let robots_txt = b"User-agent: *\nDisallow: /special/\nDisallow: /admin/";
        let robot = Robot::new("BambuMate/1.0", robots_txt).unwrap();

        assert!(!RobotsCache::check_against_robot(
            &robot,
            "https://example.com/special/page"
        ));
        assert!(!RobotsCache::check_against_robot(
            &robot,
            "https://example.com/admin/dashboard"
        ));
        assert!(RobotsCache::check_against_robot(
            &robot,
            "https://example.com/public/page"
        ));
    }

    #[test]
    fn test_robots_txt_empty_allows_all() {
        let robot = Robot::new("BambuMate/1.0", b"").unwrap();
        assert!(RobotsCache::check_against_robot(
            &robot,
            "https://example.com/anything"
        ));
    }

    #[tokio::test]
    async fn test_rate_limiter_enforces_delay() {
        let limiter = RateLimiter::new(1.0); // 1 req/sec
        let url = "https://example.com/page1";

        // First request should be immediate
        let start = Instant::now();
        limiter.wait_for_domain(url, None).await.unwrap();
        let first_elapsed = start.elapsed();
        assert!(
            first_elapsed < Duration::from_millis(100),
            "First request should be immediate, took {:?}",
            first_elapsed
        );

        // Second request to same domain should wait ~1 second
        let start = Instant::now();
        limiter.wait_for_domain(url, None).await.unwrap();
        let second_elapsed = start.elapsed();
        assert!(
            second_elapsed >= Duration::from_millis(900),
            "Second request should wait ~1s, only waited {:?}",
            second_elapsed
        );
    }

    #[tokio::test]
    async fn test_rate_limiter_different_domains() {
        let limiter = RateLimiter::new(1.0);

        // First request to domain A
        let start = Instant::now();
        limiter
            .wait_for_domain("https://example.com/page1", None)
            .await
            .unwrap();
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(100));

        // First request to domain B should also be immediate
        let start = Instant::now();
        limiter
            .wait_for_domain("https://other.com/page1", None)
            .await
            .unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(100),
            "Different domain should not wait, took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_rate_limiter_respects_extra_delay() {
        let limiter = RateLimiter::new(10.0); // 10 req/sec = 100ms interval

        let url = "https://example.com/page";
        // First request
        limiter.wait_for_domain(url, None).await.unwrap();

        // Second request with extra delay of 500ms (higher than 100ms default)
        let start = Instant::now();
        limiter
            .wait_for_domain(url, Some(Duration::from_millis(500)))
            .await
            .unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(400),
            "Extra delay should be respected, only waited {:?}",
            elapsed
        );
    }
}
