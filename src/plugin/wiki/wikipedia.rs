//! Wikipedia HTTP client — geosearch and page summaries.
//! Internal to the wiki widget.

use log::debug;

use crate::shared::http::HttpClient;
use crate::shared::http::url::urlencoded;

#[derive(Debug, Clone)]
pub(super) struct WikiArticle {
    pub title: String,
    pub extract: String,
    pub dist_m: f64,
    pub lat: f64,
    pub lon: f64,
}

pub(super) struct WikipediaClient {
    http: HttpClient,
    language: String,
}

impl WikipediaClient {
    pub(super) fn new(language: &str) -> Self {
        Self {
            http: HttpClient::new("wiki"),
            language: language.to_string(),
        }
    }

    /// Find Wikipedia articles near a coordinate.
    pub(super) fn geosearch(&self, lat: f64, lon: f64, limit: u32) -> Vec<WikiArticle> {
        let url = format!(
            "https://{}.wikipedia.org/w/api.php?action=query&list=geosearch\
             &gscoord={}|{}&gsradius=10000&gslimit={}&format=json",
            self.language, lat, lon, limit,
        );
        debug!("wiki: geosearch {}", url);

        let Some(json) = self.http.get_json::<serde_json::Value>(&url) else {
            log::warn!("wiki: geosearch fetch failed near ({}, {})", lat, lon);
            return Vec::new();
        };

        let pages = match json.pointer("/query/geosearch") {
            Some(serde_json::Value::Array(arr)) => arr,
            _ => {
                log::warn!("wiki: geosearch response missing /query/geosearch array");
                return Vec::new();
            }
        };

        struct PageInfo {
            title: String,
            dist_m: f64,
            lat: f64,
            lon: f64,
        }

        let page_infos: Vec<PageInfo> = pages
            .iter()
            .filter_map(|p| {
                Some(PageInfo {
                    title: p.get("title")?.as_str()?.to_string(),
                    dist_m: p.get("dist")?.as_f64()?,
                    lat: p.get("lat")?.as_f64()?,
                    lon: p.get("lon")?.as_f64()?,
                })
            })
            .collect();

        if page_infos.is_empty() {
            return Vec::new();
        }

        let titles: Vec<String> = page_infos.iter().map(|p| p.title.clone()).collect();
        let extracts = self.fetch_extracts(&titles);

        page_infos
            .into_iter()
            .map(|p| {
                let extract = extracts.get(&p.title).cloned().unwrap_or_default();
                WikiArticle {
                    title: p.title,
                    extract,
                    dist_m: p.dist_m,
                    lat: p.lat,
                    lon: p.lon,
                }
            })
            .collect()
    }

    fn fetch_extracts(&self, titles: &[String]) -> std::collections::HashMap<String, String> {
        let titles_param = titles.join("|");
        let url = format!(
            "https://{}.wikipedia.org/w/api.php?action=query&prop=extracts\
             &exintro=1&explaintext=1&exsentences=5&titles={}&format=json",
            self.language,
            urlencoded(&titles_param),
        );
        debug!("wiki: extracts {}", url);

        let Some(json) = self.http.get_json::<serde_json::Value>(&url) else {
            log::warn!("wiki: extracts fetch failed ({} titles)", titles.len());
            return std::collections::HashMap::new();
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
}
