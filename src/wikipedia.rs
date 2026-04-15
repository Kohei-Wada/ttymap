//! Wikipedia API client — geosearch and page summaries.

use log::debug;

const USER_AGENT: &str = "termap/0.1.0";
const TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Clone)]
pub struct WikiArticle {
    pub title: String,
    pub extract: String,
    pub dist_m: f64,
    pub lat: f64,
    pub lon: f64,
}

pub struct WikipediaClient {
    client: reqwest::blocking::Client,
    language: String,
}

impl WikipediaClient {
    pub fn new(language: &str) -> Option<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
            .build()
            .ok()?;
        Some(Self { client, language: language.to_string() })
    }

    /// Find Wikipedia articles near a coordinate.
    pub fn geosearch(&self, lat: f64, lon: f64, limit: u32) -> Vec<WikiArticle> {
        let url = format!(
            "https://{}.wikipedia.org/w/api.php?action=query&list=geosearch\
             &gscoord={}|{}&gsradius=10000&gslimit={}&format=json",
            self.language, lat, lon, limit,
        );
        debug!("wikipedia: geosearch {}", url);

        let json: serde_json::Value = match self.get_json(&url) {
            Some(v) => v,
            None => return Vec::new(),
        };

        let pages = match json.pointer("/query/geosearch") {
            Some(serde_json::Value::Array(arr)) => arr,
            _ => return Vec::new(),
        };

        struct PageInfo {
            title: String,
            dist_m: f64,
            lat: f64,
            lon: f64,
        }

        let page_infos: Vec<PageInfo> = pages.iter()
            .filter_map(|p| {
                Some(PageInfo {
                    title: p.get("title")?.as_str()?.to_string(),
                    dist_m: p.get("dist")?.as_f64()?,
                    lat: p.get("lat")?.as_f64()?,
                    lon: p.get("lon")?.as_f64()?,
                })
            })
            .collect();

        if page_infos.is_empty() { return Vec::new(); }

        let titles: Vec<String> = page_infos.iter().map(|p| p.title.clone()).collect();
        let extracts = self.fetch_extracts(&titles);

        page_infos.into_iter().map(|p| {
            let extract = extracts.get(&p.title).cloned().unwrap_or_default();
            WikiArticle { title: p.title, extract, dist_m: p.dist_m, lat: p.lat, lon: p.lon }
        }).collect()
    }

    fn fetch_extracts(&self, titles: &[String]) -> std::collections::HashMap<String, String> {
        let titles_param = titles.join("|");
        let url = format!(
            "https://{}.wikipedia.org/w/api.php?action=query&prop=extracts\
             &exintro=1&explaintext=1&exsentences=2&titles={}&format=json",
            self.language, urlencoded(&titles_param),
        );
        debug!("wikipedia: extracts {}", url);

        let json: serde_json::Value = match self.get_json(&url) {
            Some(v) => v,
            None => return std::collections::HashMap::new(),
        };

        let mut result = std::collections::HashMap::new();
        if let Some(pages) = json.pointer("/query/pages")
            && let Some(obj) = pages.as_object()
        {
            for (_, page) in obj {
                if let Some(title) = page.get("title").and_then(|t| t.as_str())
                    && let Some(extract) = page.get("extract").and_then(|e| e.as_str())
                {
                    result.insert(title.to_string(), extract.to_string());
                }
            }
        }

        result
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Option<T> {
        let response = match self.client.get(url).send() {
            Ok(r) => r,
            Err(e) => {
                debug!("wikipedia: request error: {}", e);
                return None;
            }
        };
        if !response.status().is_success() {
            debug!("wikipedia: status {}", response.status());
            return None;
        }
        match response.json() {
            Ok(j) => Some(j),
            Err(e) => {
                debug!("wikipedia: parse error: {}", e);
                None
            }
        }
    }
}

fn urlencoded(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    out
}
