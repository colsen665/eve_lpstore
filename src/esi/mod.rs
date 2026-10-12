pub mod corporations;
pub mod error;
pub mod loyalty;
pub mod markets;
pub mod retry;
pub mod universe;

use error::{EsiError, SharedError};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{OnceCell, Semaphore};

type PageResult = Result<(serde_json::Value, Option<u32>), EsiError>;

/// Statistics about outstanding API calls. The default is a client that has made no requests.
#[derive(Debug, Clone, Default)]
pub struct ClientStats {
    pub in_flight: usize,
    pub waiting: usize,
    pub rate_limit_remain: Option<u32>,
    pub rate_limit_reset: Option<u32>,
    pub total_requests: u64,
    pub total_errors: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_writes: u64,
}

/// Counts a request as waiting for a permit until it is dropped.
struct WaitingGuard(Arc<Mutex<ClientStats>>);

impl WaitingGuard {
    fn new(stats: Arc<Mutex<ClientStats>>) -> Self {
        stats.lock().unwrap().waiting += 1;
        Self(stats)
    }
}

impl Drop for WaitingGuard {
    fn drop(&mut self) {
        self.0.lock().unwrap().waiting -= 1;
    }
}

/// Counts a request as in flight until it is dropped.
struct InFlightGuard(Arc<Mutex<ClientStats>>);

impl InFlightGuard {
    fn new(stats: Arc<Mutex<ClientStats>>) -> Self {
        stats.lock().unwrap().in_flight += 1;
        Self(stats)
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.lock().unwrap().in_flight -= 1;
    }
}

const DEFAULT_BASE_URL: &str = "https://esi.evetech.net";
const DEFAULT_DATASOURCE: &str = "tranquility";
const DEFAULT_LANGUAGE: &str = "en";

/// Receives the client's verbose log lines.
#[derive(Clone)]
pub struct LogSink(Arc<dyn Fn(String) + Send + Sync>);

impl LogSink {
    fn send(&self, message: String) {
        (self.0)(message)
    }
}

impl std::fmt::Debug for LogSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LogSink")
    }
}

/// Configuration for the ESI client.
#[derive(Debug, Clone)]
pub struct EsiConfig {
    pub base_url: String,
    pub datasource: String,
    pub language: String,
    pub token: Option<String>,
    pub user_agent: String,
    pub timeout: Duration,
    pub cache_dir: Option<PathBuf>,
    pub expire: bool,
    pub verbose: bool,
    pub cache_expiry_offset: Duration,
    pub max_inflight: usize,
    pub log: Option<LogSink>,
}

impl Default for EsiConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            datasource: DEFAULT_DATASOURCE.to_string(),
            language: DEFAULT_LANGUAGE.to_string(),
            token: None,
            user_agent: "eve_lpstore/0.1.0".to_string(),
            timeout: Duration::from_secs(30),
            cache_dir: None,
            expire: false,
            verbose: false,
            cache_expiry_offset: Duration::ZERO,
            max_inflight: 4,
            log: None,
        }
    }
}

/// Builder for [`EsiClient`].
pub struct EsiClientBuilder {
    config: EsiConfig,
}

impl EsiClientBuilder {
    pub fn new() -> Self {
        Self {
            config: EsiConfig::default(),
        }
    }

    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.config.base_url = url.into();
        self
    }

    pub fn datasource(mut self, ds: impl Into<String>) -> Self {
        self.config.datasource = ds.into();
        self
    }

    pub fn language(mut self, lang: impl Into<String>) -> Self {
        self.config.language = lang.into();
        self
    }

    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.config.token = Some(token.into());
        self
    }

    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.config.user_agent = ua.into();
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Where to write cached responses. A path rather than a string, because the platform locations
    /// carry spaces and backslashes on Windows.
    pub fn cache_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.config.cache_dir = Some(dir.into());
        self
    }

    pub fn expire(mut self, expire: bool) -> Self {
        self.config.expire = expire;
        self
    }

    pub fn verbose(mut self, verbose: bool) -> Self {
        self.config.verbose = verbose;
        self
    }

    pub fn cache_expiry_offset(mut self, offset: Duration) -> Self {
        self.config.cache_expiry_offset = offset;
        self
    }

    pub fn max_inflight(mut self, max_inflight: usize) -> Self {
        self.config.max_inflight = max_inflight;
        self
    }

    pub fn log(mut self, sink: impl Fn(String) + Send + Sync + 'static) -> Self {
        self.config.log = Some(LogSink(Arc::new(sink)));
        self
    }

    pub fn build(self) -> EsiClient {
        EsiClient::new(self.config)
    }
}

impl Default for EsiClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// The main ESI client. Provides typed access to EVE Online's ESI API.
pub struct EsiClient {
    http: reqwest::Client,
    config: EsiConfig,
    stats: Arc<Mutex<ClientStats>>,
    inflight: Mutex<HashMap<String, Arc<OnceCell<Arc<PageResult>>>>>,
    request_permits: Semaphore,
    cache_index: Mutex<CacheIndex>,
}

impl EsiClient {
    pub fn builder() -> EsiClientBuilder {
        EsiClientBuilder::new()
    }

    pub fn new(config: EsiConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .expect("failed to build HTTP client");
        let request_permits = Semaphore::new(config.max_inflight);
        Self {
            http,
            config,
            stats: Arc::new(Mutex::new(ClientStats::default())),
            inflight: Mutex::new(HashMap::new()),
            request_permits,
            cache_index: Mutex::new(CacheIndex::default()),
        }
    }

    /// Sends a verbose log line to the configured sink, if there is one.
    fn verbose_log(&self, message: String) {
        if let Some(sink) = &self.config.log {
            sink.send(message);
        }
    }

    /// Returns a snapshot of the current API call statistics.
    pub fn status(&self) -> ClientStats {
        self.stats.lock().unwrap().clone()
    }

    /// Spawns a task that passes a snapshot of the stats to `on_status` once per second.
    pub fn spawn_status_monitor<F>(&self, on_status: F) -> tokio::task::JoinHandle<()>
    where
        F: Fn(&ClientStats) + Send + 'static,
    {
        let stats = self.stats.clone();
        tokio::spawn(async move {
            loop {
                let snapshot = stats.lock().unwrap().clone();
                on_status(&snapshot);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        })
    }

    /// Returns a reference to the corporations endpoint module.
    pub fn corporations(&self) -> corporations::CorporationsEndpoint<'_> {
        corporations::CorporationsEndpoint { client: self }
    }

    /// Returns a reference to the loyalty endpoint module.
    pub fn loyalty(&self) -> loyalty::LoyaltyEndpoint<'_> {
        loyalty::LoyaltyEndpoint { client: self }
    }

    /// Returns a reference to the markets endpoint module.
    pub fn markets(&self) -> markets::MarketsEndpoint<'_> {
        markets::MarketsEndpoint { client: self }
    }

    /// Returns a reference to the universe endpoint module.
    pub fn universe(&self) -> universe::UniverseEndpoint<'_> {
        universe::UniverseEndpoint { client: self }
    }

    /// Returns a reference to the expenses endpoint module.
    pub fn expenses(&self) -> crate::expenses::ExpensesEndpoint<'_> {
        crate::expenses::ExpensesEndpoint { client: self }
    }

    /// Perform a GET request against the ESI API, deserializing the response into `T`.
    ///
    /// Handles rate-limit headers, retries on 420/5xx with exponential backoff,
    /// and automatic pagination when the `X-Pages` header is present.
    pub async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, EsiError> {
        let url = self.build_url(path, query)?;
        self.fetch_all_pages(&url).await
    }

    /// Build a full URL from a path and query parameters.
    fn build_url(&self, path: &str, query: &[(&str, String)]) -> Result<url::Url, EsiError> {
        let mut url = url::Url::parse(&format!("{}/{}", self.config.base_url, path))?;

        // Always include datasource and language
        url.query_pairs_mut()
            .append_pair("datasource", &self.config.datasource)
            .append_pair("language", &self.config.language);

        for (key, value) in query {
            url.query_pairs_mut().append_pair(key, value);
        }

        Ok(url)
    }

    /// Fetch every page of a paginated endpoint and join the results.
    ///
    /// The page count comes from the first page alone, cached or not. Following each page's own
    /// count let a cached page, which used to report itself as the only one, end the walk early.
    async fn fetch_all_pages<T: DeserializeOwned>(&self, url: &url::Url) -> Result<T, EsiError> {
        let (body, pages) = self.fetch_single_page(url).await?;
        let serde_json::Value::Array(mut items) = body else {
            return Ok(serde_json::from_value(body)?);
        };
        for page in 2..=pages.unwrap_or(1) {
            let (body, _) = self.fetch_single_page(&with_page(url, page)).await?;
            items.extend(serde_json::from_value::<Vec<serde_json::Value>>(body)?);
        }
        Ok(serde_json::from_value(serde_json::Value::Array(items))?)
    }

    /// Fetch a single page. Checks the cache first, then shares one in-flight request per URL.
    async fn fetch_single_page(
        &self,
        url: &url::Url,
    ) -> Result<(serde_json::Value, Option<u32>), EsiError> {
        if let Some(cached) = self.check_cache(url).await {
            self.stats.lock().unwrap().cache_hits += 1;
            return Ok(cached);
        }
        self.stats.lock().unwrap().cache_misses += 1;

        let key = url.as_str().to_string();
        let cell = {
            let mut inflight = self.inflight.lock().unwrap();
            inflight
                .entry(key.clone())
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone()
        };

        let shared = cell
            .get_or_init(|| async { Arc::new(self.request_page(url).await) })
            .await
            .clone();

        {
            let mut inflight = self.inflight.lock().unwrap();
            if inflight.get(&key).is_some_and(|c| Arc::ptr_eq(c, &cell)) {
                inflight.remove(&key);
            }
        }

        match shared.as_ref() {
            Ok((body, pages)) => Ok((body.clone(), *pages)),
            Err(e) => Err(EsiError::Shared(SharedError::from(e))),
        }
    }

    /// Perform the HTTP request for one page, tracking in-flight and error stats.
    async fn request_page(&self, url: &url::Url) -> PageResult {
        let result = self.fetch_single_page_inner(url).await;

        // Update stats
        {
            let mut stats = self.stats.lock().unwrap();
            match &result {
                Ok(_) => stats.total_requests += 1,
                Err(_) => stats.total_errors += 1,
            }
        }

        result
    }

    /// Derive cache entry type and ID from URL path and query parameters.
    fn cache_entry_info(&self, url: &url::Url) -> Option<(String, String)> {
        let path = url.path();
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            return None;
        }
        let entry_type = segments[0].to_string();
        let mut entry_id = segments[1..].join("_");

        // Append query parameters to entry ID
        if url.query().is_some_and(|query| !query.is_empty()) {
            let params: Vec<String> = url
                .query_pairs()
                .map(|(k, v)| format!("{}_{}", k, v))
                .collect();
            entry_id.push('_');
            entry_id.push_str(&params.join("_"));
        }

        Some((entry_type, entry_id))
    }

    /// Check if a valid cache entry exists for the given URL, and return its body and page count.
    async fn check_cache(&self, url: &url::Url) -> Option<(serde_json::Value, Option<u32>)> {
        let cache_dir = match &self.config.cache_dir {
            Some(dir) => dir,
            None => {
                if self.config.verbose {
                    self.verbose_log(format!("[CACHE MISS] {} (caching disabled)", url.path()));
                }
                return None;
            }
        };
        if self.config.expire {
            if self.config.verbose {
                self.verbose_log(format!("[CACHE MISS] {} (--expire forced)", url.path()));
            }
            return None;
        }

        let (entry_type, entry_id) = match self.cache_entry_info(url) {
            Some(info) => info,
            None => {
                if self.config.verbose {
                    self.verbose_log(format!("[CACHE MISS] {} (no entry info)", url.path()));
                }
                return None;
            }
        };
        let key = format!("{}_{}", entry_type, entry_id);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        let mut seen = MissReasons::default();

        // The index answers a hit without listing the directory. Only a miss lists it again, which
        // is how a file another process wrote since the last listing is found.
        let mut found = self.read_indexed(cache_dir, &key, now, &mut seen).await;
        let mut listed = true;
        if found.is_none() {
            listed = self.list_cache(cache_dir).await;
            found = self.read_indexed(cache_dir, &key, now, &mut seen).await;
        }

        if let Some((value, effective_expiry)) = found {
            if self.config.verbose {
                let remaining = effective_expiry - now;
                let expiry_dt = chrono::DateTime::from_timestamp(effective_expiry as i64, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                    .unwrap_or_else(|| effective_expiry.to_string());
                self.verbose_log(format!(
                    "[CACHE HIT] {} → expires {} (in {}s)",
                    url.path(),
                    expiry_dt,
                    remaining
                ));
            }
            return Some(value);
        }

        if self.config.verbose {
            if !listed {
                self.verbose_log(format!("[CACHE MISS] {} (cache dir not found)", url.path()));
                return None;
            }
            let reason = if seen.expired {
                "entry expired"
            } else if seen.corrupt {
                "entry corrupt"
            } else {
                "no matching entry"
            };
            let expected_file = format!("{}_<expiry_timestamp>.json", key);
            self.verbose_log(format!(
                "[CACHE MISS] {} ({}) — expected file: {}",
                url.path(),
                reason,
                expected_file
            ));
        }
        None
    }

    /// Reads the newest usable file the index holds for `key`, with the time it stops being
    /// usable. Every expired file for the entry is deleted on the way, a file that has gone from
    /// disk is dropped from the index, and one that does not read or parse is passed over.
    async fn read_indexed(
        &self,
        dir: &Path,
        key: &str,
        now: u64,
        seen: &mut MissReasons,
    ) -> Option<((serde_json::Value, Option<u32>), u64)> {
        let expiries = self.cache_index.lock().unwrap().expiries(key);
        let offset = self.config.cache_expiry_offset.as_secs();
        let (usable, expired): (Vec<u64>, Vec<u64>) = expiries
            .into_iter()
            .partition(|expiry| now < expiry + offset);
        for expiry in expired {
            seen.expired = true;
            let _ = tokio::fs::remove_file(dir.join(cache_file_name(key, expiry))).await;
            self.cache_index.lock().unwrap().remove(key, expiry);
        }
        for expiry in usable {
            match tokio::fs::read_to_string(dir.join(cache_file_name(key, expiry))).await {
                Ok(content) => match serde_json::from_str(&content) {
                    Ok(value) => return Some((unwrap_cached(value), expiry + offset)),
                    Err(_) => seen.corrupt = true,
                },
                // Removed by something else since the index saw it, such as --clear-cache.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    self.cache_index.lock().unwrap().remove(key, expiry);
                }
                Err(_) => seen.corrupt = true,
            }
        }
        None
    }

    /// Lists the cache directory into the index, keeping what the index already holds, so a file
    /// this client wrote while the listing ran is not dropped. `false` when the directory cannot
    /// be read, which is the state before the first write creates it.
    async fn list_cache(&self, dir: &Path) -> bool {
        let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
            return false;
        };
        let mut found = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            if let Some((key, expiry)) = name.to_str().and_then(parse_cache_file_name) {
                found.push((key.to_string(), expiry));
            }
        }
        let mut index = self.cache_index.lock().unwrap();
        index.listings += 1;
        for (key, expiry) in found {
            index.insert(key, expiry);
        }
        true
    }

    /// Caches a response that has already arrived. A write that fails is logged rather than
    /// returned: the body is in hand, so a cache that cannot take it costs the next run a fetch
    /// instead of costing this one the result.
    async fn store(
        &self,
        url: &url::Url,
        body: &serde_json::Value,
        pages: Option<u32>,
        expires_header: Option<&str>,
    ) {
        if let Err(error) = self.write_cache(url, body, pages, expires_header).await {
            self.verbose_log(format!("[CACHE WRITE FAILED] {}: {}", url.path(), error));
        }
    }

    /// Write a successful response to the cache, with the page count it arrived with.
    async fn write_cache(
        &self,
        url: &url::Url,
        body: &serde_json::Value,
        pages: Option<u32>,
        expires_header: Option<&str>,
    ) -> Result<(), EsiError> {
        let cache_dir = match &self.config.cache_dir {
            Some(dir) => dir,
            None => return Ok(()),
        };

        let (entry_type, entry_id) = match self.cache_entry_info(url) {
            Some(info) => info,
            None => return Ok(()),
        };

        // Parse Expires header to get unix timestamp
        let expiry = match expires_header {
            Some(header) => {
                // Parse HTTP date format (RFC 1123 / RFC 2822)
                let dt = chrono::DateTime::parse_from_rfc2822(header).map_err(io::Error::other)?;
                dt.timestamp() as u64
            }
            None => {
                // Default: cache for 1 hour if no Expires header
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(io::Error::other)?
                    .as_secs()
                    + 3600
            }
        };

        // Ensure cache directory exists
        tokio::fs::create_dir_all(cache_dir).await.map_err(EsiError::Io)?;

        // Write cache file
        let filename = format!("{}_{}_{}.json", entry_type, entry_id, expiry);
        // Write to a temporary file, then rename it into place so readers never see a partial file
        let filepath = std::path::Path::new(cache_dir).join(&filename);
        let temp_path = std::path::Path::new(cache_dir)
            .join(format!(".{}.{}.tmp", filename, std::process::id()));
        let content = serde_json::to_string(&CachedPageRef { pages, body })?;
        tokio::fs::write(&temp_path, content).await.map_err(EsiError::Io)?;
        tokio::fs::rename(&temp_path, &filepath).await.map_err(EsiError::Io)?;
        self.stats.lock().unwrap().cache_writes += 1;
        self.cache_index
            .lock()
            .unwrap()
            .insert(format!("{}_{}", entry_type, entry_id), expiry);

        if self.config.verbose {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let remaining = expiry.saturating_sub(now);
            let expiry_dt = chrono::DateTime::from_timestamp(expiry as i64, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| expiry.to_string());
            self.verbose_log(format!(
                "[CACHE WRITE] {} → {} → expires {} (in {}s)",
                url.path(),
                filename,
                expiry_dt,
                remaining
            ));
        }

        Ok(())
    }

    /// Inner implementation of fetch_single_page without stats tracking.
    async fn fetch_single_page_inner(
        &self,
        url: &url::Url,
    ) -> Result<(serde_json::Value, Option<u32>), EsiError> {
        let mut attempt = 0u32;
        loop {
            let mut headers = HeaderMap::new();
            headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
            headers.insert(USER_AGENT, HeaderValue::from_str(&self.config.user_agent)?);
            if let Some(ref token) = self.config.token {
                headers.insert(
                    AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {}", token))?,
                );
            }

            let waiting = WaitingGuard::new(self.stats.clone());
            let permit = self
                .request_permits
                .acquire()
                .await
                .expect("request semaphore is never closed");
            drop(waiting);
            let in_flight = InFlightGuard::new(self.stats.clone());
            let response = self
                .http
                .get(url.clone())
                .headers(headers)
                .send()
                .await;
            let response = response.map_err(EsiError::Http)?;

            let status = response.status();

            // Track rate limit headers
            let remain = response
                .headers()
                .get("x-esi-error-limit-remain")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u32>().ok());
            let reset = response
                .headers()
                .get("x-esi-error-limit-reset")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u32>().ok());

            // Update rate limit stats
            if let (Some(remain), Some(reset)) = (remain, reset) {
                let mut stats = self.stats.lock().unwrap();
                stats.rate_limit_remain = Some(remain);
                stats.rate_limit_reset = Some(reset);
                if remain == 0 {
                    return Err(EsiError::RateLimit { remain, reset });
                }
            }

            // Success
            if status.is_success() {
                let pages = response
                    .headers()
                    .get("x-pages")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u32>().ok());

                let expires = response
                    .headers()
                    .get("expires")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());

                let body: serde_json::Value =
                    response.json().await.map_err(EsiError::Http)?;
                drop(in_flight);
                drop(permit);

                self.store(url, &body, pages, expires.as_deref()).await;
                return Ok((body, pages));
            }

            // Retryable: 420 (error limit) or 5xx
            if status.as_u16() == 420 || status.is_server_error() {
                if attempt >= retry::MAX_RETRIES {
                    return Err(EsiError::Throttle);
                }
                drop(in_flight);
                drop(permit);
                let delay = retry::backoff_delay(attempt);
                tokio::time::sleep(delay).await;
                attempt += 1;
                continue;
            }

            // 404 on a paginated request means we've reached the end
            if status.as_u16() == 404 {
                return Ok((serde_json::Value::Array(vec![]), Some(1)));
            }

            // Non-retryable error
            let message = response.text().await.unwrap_or_default();
            return Err(EsiError::Esi {
                status: status.as_u16(),
                message,
            });
        }
    }
}

/// The cache directory's files by entry: the file name without its `_{expiry}.json` ending, mapped
/// to the expiries on disk for it. A lookup reads from here, so a hit costs no directory listing.
/// It is filled by listing the directory on a miss and by the client's own writes.
#[derive(Debug, Default)]
struct CacheIndex {
    entries: HashMap<String, BTreeSet<u64>>,
    /// How many times the directory has been listed, which is what the tests count.
    listings: u64,
}

impl CacheIndex {
    fn insert(&mut self, key: String, expiry: u64) {
        self.entries.entry(key).or_default().insert(expiry);
    }

    fn remove(&mut self, key: &str, expiry: u64) {
        if let Some(expiries) = self.entries.get_mut(key) {
            expiries.remove(&expiry);
            if expiries.is_empty() {
                self.entries.remove(key);
            }
        }
    }

    /// The expiries on disk for `key`, latest first, so the newest response is read first.
    fn expiries(&self, key: &str) -> Vec<u64> {
        self.entries
            .get(key)
            .map(|expiries| expiries.iter().rev().copied().collect())
            .unwrap_or_default()
    }
}

/// Why a lookup found nothing usable, for the verbose miss line.
#[derive(Debug, Default)]
struct MissReasons {
    expired: bool,
    corrupt: bool,
}

/// The name a cached response is written under.
fn cache_file_name(key: &str, expiry: u64) -> String {
    format!("{}_{}.json", key, expiry)
}

/// Splits a cache file name into its entry and its expiry, or `None` for a file that is not a
/// cached response, such as the temporary file of a write in progress. A sibling such as page 2
/// keeps its own entry, `..._page_2`, so it never answers for page 1.
fn parse_cache_file_name(name: &str) -> Option<(&str, u64)> {
    let (key, expiry) = name.strip_suffix(".json")?.rsplit_once('_')?;
    Some((key, expiry.parse().ok()?))
}

/// `url` asking for `page`, with any page it already named replaced.
fn with_page(url: &url::Url, page: u32) -> url::Url {
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| key != "page")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    let mut paged = url.clone();
    paged
        .query_pairs_mut()
        .clear()
        .extend_pairs(pairs)
        .append_pair("page", &page.to_string());
    paged
}

/// A cache file as it is written: the body, and the `X-Pages` count the response carried.
#[derive(Serialize)]
struct CachedPageRef<'a> {
    pages: Option<u32>,
    body: &'a serde_json::Value,
}

/// Splits a cache file into its body and page count. A file written before the count was stored
/// holds the bare body, and reads as one page, which is how every cached page used to read.
fn unwrap_cached(value: serde_json::Value) -> (serde_json::Value, Option<u32>) {
    match value {
        serde_json::Value::Object(mut file)
            if file.len() == 2 && file.contains_key("pages") && file.contains_key("body") =>
        {
            let pages = file
                .get("pages")
                .and_then(serde_json::Value::as_u64)
                .and_then(|pages| u32::try_from(pages).ok());
            let body = file.remove("body").unwrap_or_default();
            (body, pages)
        }
        legacy => (legacy, Some(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("eve_lpstore_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn page_one_is_found_past_its_page_siblings() {
        let dir = scratch_dir("page_one");
        let expiry = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let base = "markets_10000002_orders_datasource_tranquility_language_en";
        std::fs::write(dir.join(format!("{}_{}.json", base, expiry)), "[1]").unwrap();
        for page in 2..12 {
            let name = format!("{}_page_{}_{}.json", base, page, expiry);
            std::fs::write(dir.join(name), format!("[{}]", page)).unwrap();
        }

        let client = EsiClient::builder().cache_dir(dir.to_path_buf()).build();
        let url = url::Url::parse(
            "https://esi.evetech.net/markets/10000002/orders/?datasource=tranquility&language=en",
        )
        .unwrap();

        assert_eq!(
            client.check_cache(&url).await,
            Some((serde_json::json!([1]), Some(1)))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn in_an_hour() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600
    }

    /// A client that reads `dir` and has no server to reach, so a test that should be answered
    /// from the cache fails rather than quietly fetching.
    fn offline_client(dir: &std::path::Path) -> EsiClient {
        EsiClient::builder()
            .base_url("http://127.0.0.1:9")
            .cache_dir(dir.to_path_buf())
            .build()
    }

    const ORDERS: &str = "markets_10000002_orders_datasource_tranquility_language_en";

    #[tokio::test]
    async fn a_cached_multi_page_response_returns_every_page() {
        // Page one used to come back from the cache as the only page, so the walk stopped there
        // with pages two onward cached beside it.
        let dir = scratch_dir("every_page");
        let expiry = in_an_hour();
        for (page, name) in [
            (1, format!("{ORDERS}_{expiry}.json")),
            (2, format!("{ORDERS}_page_2_{expiry}.json")),
            (3, format!("{ORDERS}_page_3_{expiry}.json")),
        ] {
            let body = format!(r#"{{"pages":3,"body":[{page}]}}"#);
            std::fs::write(dir.join(name), body).unwrap();
        }

        let orders: Vec<i32> = offline_client(&dir)
            .get("markets/10000002/orders/", &[])
            .await
            .expect("every page is cached");
        assert_eq!(orders, [1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_cache_file_without_a_page_count_reads_as_one_page() {
        // Files written before the count was stored hold the bare body, so they keep reading the
        // way they always did rather than forcing a refetch.
        let dir = scratch_dir("legacy_page");
        let expiry = in_an_hour();
        std::fs::write(dir.join(format!("{ORDERS}_{expiry}.json")), "[1]").unwrap();
        std::fs::write(dir.join(format!("{ORDERS}_page_2_{expiry}.json")), "[2]").unwrap();

        let orders: Vec<i32> = offline_client(&dir)
            .get("markets/10000002/orders/", &[])
            .await
            .expect("page one is cached");
        assert_eq!(orders, [1]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_written_page_keeps_its_page_count() {
        let dir = scratch_dir("written_count");
        let client = offline_client(&dir);
        let url = client.build_url("markets/10000002/orders/", &[]).unwrap();
        let body = serde_json::json!([{"order_id": 1}]);
        client
            .write_cache(&url, &body, Some(7), None)
            .await
            .unwrap();
        assert_eq!(client.check_cache(&url).await, Some((body, Some(7))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_object_body_is_not_mistaken_for_a_page_count() {
        // A response that is an object, such as a type's details, is cached inside the same
        // wrapper, and only the wrapper's two fields are unwrapped.
        let dir = scratch_dir("object_body");
        let client = offline_client(&dir);
        let url = client.build_url("universe/types/34/", &[]).unwrap();
        let body = serde_json::json!({"pages": 1, "body": "x", "type_id": 34});
        client.write_cache(&url, &body, None, None).await.unwrap();
        assert_eq!(client.check_cache(&url).await, Some((body, None)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_page_parameter_is_replaced_not_repeated() {
        let url = url::Url::parse("https://esi.evetech.net/x/?datasource=t&page=2").unwrap();
        assert_eq!(
            with_page(&url, 3).as_str(),
            "https://esi.evetech.net/x/?datasource=t&page=3"
        );
    }

    fn listings(client: &EsiClient) -> u64 {
        client.cache_index.lock().unwrap().listings
    }

    fn orders_url(client: &EsiClient, type_id: i32) -> url::Url {
        client
            .build_url(
                "markets/10000002/orders/",
                &[("type_id", type_id.to_string())],
            )
            .unwrap()
    }

    /// The file name a cached order lookup for `type_id` is written under.
    fn orders_file(type_id: i32, expiry: u64) -> String {
        format!("{ORDERS}_type_id_{type_id}_{expiry}.json")
    }

    #[tokio::test]
    async fn a_hit_after_the_first_lookup_lists_nothing() {
        // The cost this index removes: every lookup used to read the whole directory.
        let dir = scratch_dir("index_hits");
        let expiry = in_an_hour();
        for type_id in [34, 35, 36] {
            std::fs::write(dir.join(orders_file(type_id, expiry)), "[1]").unwrap();
        }
        let client = offline_client(&dir);
        for type_id in [34, 35, 36, 34] {
            let url = orders_url(&client, type_id);
            assert!(client.check_cache(&url).await.is_some(), "{type_id}");
        }
        assert_eq!(
            listings(&client),
            1,
            "only the first lookup listed the directory"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_file_another_process_writes_is_found_on_the_next_miss() {
        let dir = scratch_dir("index_other_process");
        let client = offline_client(&dir);
        let url = orders_url(&client, 34);
        assert_eq!(client.check_cache(&url).await, None);
        // Written behind the client's back, as a second view or a CLI run would.
        std::fs::write(dir.join(orders_file(34, in_an_hour())), "[7]").unwrap();
        assert_eq!(
            client.check_cache(&url).await,
            Some((serde_json::json!([7]), Some(1)))
        );
        assert_eq!(listings(&client), 2, "the miss listed again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_clients_own_write_is_found_without_a_listing() {
        let dir = scratch_dir("index_own_write");
        let client = offline_client(&dir);
        let url = orders_url(&client, 34);
        client
            .write_cache(&url, &serde_json::json!([3]), Some(1), None)
            .await
            .unwrap();
        assert_eq!(
            client.check_cache(&url).await,
            Some((serde_json::json!([3]), Some(1)))
        );
        assert_eq!(listings(&client), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_file_removed_by_something_else_is_a_miss_and_leaves_the_index() {
        let dir = scratch_dir("index_removed");
        let file = dir.join(orders_file(34, in_an_hour()));
        std::fs::write(&file, "[1]").unwrap();
        let client = offline_client(&dir);
        let url = orders_url(&client, 34);
        assert!(client.check_cache(&url).await.is_some());
        std::fs::remove_file(&file).unwrap();
        assert_eq!(client.check_cache(&url).await, None);
        assert!(
            client
                .cache_index
                .lock()
                .unwrap()
                .expiries(&format!("{ORDERS}_type_id_34"))
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_newest_file_answers_and_expired_ones_are_deleted() {
        let dir = scratch_dir("index_newest");
        let now = in_an_hour() - 3600;
        let expired = dir.join(orders_file(34, now - 10));
        let older = dir.join(orders_file(34, now + 600));
        let newer = dir.join(orders_file(34, now + 1200));
        std::fs::write(&expired, "[0]").unwrap();
        std::fs::write(&older, "[1]").unwrap();
        std::fs::write(&newer, "[2]").unwrap();
        let client = offline_client(&dir);
        let url = orders_url(&client, 34);
        assert_eq!(
            client.check_cache(&url).await,
            Some((serde_json::json!([2]), Some(1)))
        );
        assert!(!expired.exists(), "the expired file is deleted");
        assert!(older.exists(), "a usable older file is left alone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_missing_cache_directory_is_a_miss() {
        let dir =
            std::env::temp_dir().join(format!("eve_lpstore_no_cache_dir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let client = offline_client(&dir);
        assert_eq!(client.check_cache(&orders_url(&client, 34)).await, None);
        assert_eq!(listings(&client), 0, "nothing could be listed");
    }

    #[test]
    fn only_response_files_name_an_entry() {
        assert_eq!(
            parse_cache_file_name("markets_1_orders_datasource_t_1700000000.json"),
            Some(("markets_1_orders_datasource_t", 1700000000))
        );
        assert_eq!(
            parse_cache_file_name("markets_1_orders_page_2_1700000000.json"),
            Some(("markets_1_orders_page_2", 1700000000)),
            "page 2 is its own entry"
        );
        assert_eq!(
            parse_cache_file_name(".markets_1_orders_1700000000.json.4242.tmp"),
            None
        );
        assert_eq!(parse_cache_file_name("notes.txt"), None);
        assert_eq!(parse_cache_file_name("markets_1_orders_soon.json"), None);
    }

    #[tokio::test]
    async fn a_failed_cache_write_is_logged_rather_than_returned() {
        // An Expires header that is not a date fails the write. `store` is what the fetch path
        // calls with a body already in hand, and it has no error to hand back.
        let dir = scratch_dir("write_failure");
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = lines.clone();
        let client = EsiClient::builder()
            .base_url("http://127.0.0.1:9")
            .cache_dir(dir.clone())
            .log(move |line| sink.lock().unwrap().push(line))
            .build();
        let url = client.build_url("markets/10000002/orders/", &[]).unwrap();
        client
            .store(&url, &serde_json::json!([1]), Some(1), Some("not a date"))
            .await;
        assert_eq!(client.check_cache(&url).await, None, "nothing was written");
        let lines = lines.lock().unwrap().clone();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("[CACHE WRITE FAILED] /markets/10000002/orders/:"),
            "{}",
            lines[0]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
