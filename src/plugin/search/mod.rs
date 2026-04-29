//! Search plugin — Nominatim forward geocoding as a palette provider.
//!
//! Unlike most plugins, search isn't a `Component`: a one-shot
//! "type → debounced fetch → pick → jump" workflow fits the palette's
//! universal-picker shape. The plugin's `register` simply binds `/`
//! (and a `Search location` palette entry) to push the palette
//! pre-loaded with [`SearchProvider`].
//!
//! Debounce: typing pauses for `DEBOUNCE` before a request is sent,
//! so Nominatim's free endpoint isn't hammered while the user types.

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::app::AppMsg;
use crate::compositor::{Context, Registrar};
use crate::palette::PaletteComponent;
use crate::palette::provider::{PaletteAction, PaletteItem, PaletteProvider, SubmitMode};
use crate::plugin_api::AsyncJob;
use crate::plugin_api::nominatim::{NominatimClient, SearchResult};

const DEBOUNCE: Duration = Duration::from_millis(400);

/// Wire the search provider into the registrar:
/// - `/` activation → push palette pre-loaded with [`SearchProvider`]
/// - palette entry "Search location" doing the same
pub fn register(client: Arc<NominatimClient>, r: &mut Registrar) {
    let client_for_key = client.clone();
    r.bind(KeyCode::Char('/'), KeyModifiers::NONE, move |_| {
        PaletteComponent::with_provider(Box::new(SearchProvider::new(client_for_key.clone())))
    });
    r.add_spawn("Search location", "/", move |_| {
        PaletteComponent::with_provider(Box::new(SearchProvider::new(client.clone())))
    });
}

pub struct SearchProvider {
    client: Arc<NominatimClient>,
    job: AsyncJob<Vec<SearchResult>>,
    pending: bool,
    last_query: String,
    candidates: Vec<SearchResult>,
    items: Vec<PaletteItem>,
}

impl SearchProvider {
    pub fn new(client: Arc<NominatimClient>) -> Self {
        Self {
            client,
            job: AsyncJob::new(),
            pending: false,
            last_query: String::new(),
            candidates: Vec::new(),
            items: Vec::new(),
        }
    }

    fn rebuild_items(&mut self) {
        self.items = self
            .candidates
            .iter()
            .map(|c| PaletteItem {
                label: c.name.clone(),
                hint: String::new(),
            })
            .collect();
    }
}

impl PaletteProvider for SearchProvider {
    fn prompt(&self) -> &str {
        "/"
    }

    fn filter(&mut self, query: &str) {
        if query.trim().is_empty() {
            self.candidates.clear();
            self.items.clear();
            self.pending = false;
            self.last_query.clear();
            return;
        }
        if query == self.last_query && (self.pending || !self.candidates.is_empty()) {
            return;
        }
        self.last_query = query.to_string();
        self.candidates.clear();
        self.items.clear();
        let client = self.client.clone();
        let q = query.to_string();
        self.job.spawn(move || client.search(&q));
        self.pending = true;
    }

    fn items(&self) -> &[PaletteItem] {
        &self.items
    }

    fn execute(&mut self, idx: usize, _ctx: &Context) -> PaletteAction {
        match self.candidates.get(idx) {
            Some(c) => PaletteAction::Run(vec![AppMsg::Jump(c.location)]),
            None => PaletteAction::Close,
        }
    }

    fn submit_mode(&self) -> SubmitMode {
        SubmitMode::Debounced(DEBOUNCE)
    }

    fn poll(&mut self) {
        if let Some(results) = self.job.poll() {
            self.candidates = results;
            self.pending = false;
            self.rebuild_items();
        }
    }

    fn is_loading(&self) -> bool {
        self.pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::LonLat;
    use crate::theme::ThemeId;

    fn ctx() -> Context {
        Context {
            center: LonLat { lon: 0.0, lat: 0.0 },
            theme_id: ThemeId::Dark,
            cursor: None,
        }
    }

    #[test]
    fn empty_query_clears_state() {
        let mut p = SearchProvider::new(Arc::new(NominatimClient::new()));
        p.candidates.push(SearchResult {
            name: "stale".to_string(),
            location: LonLat { lon: 0.0, lat: 0.0 },
        });
        p.rebuild_items();
        p.last_query = "stale".to_string();

        p.filter("");
        assert!(p.candidates.is_empty());
        assert!(p.items.is_empty());
        assert!(!p.pending);
    }

    #[test]
    fn execute_out_of_range_closes() {
        let mut p = SearchProvider::new(Arc::new(NominatimClient::new()));
        let ctx = ctx();
        assert!(matches!(p.execute(99, &ctx), PaletteAction::Close));
    }

    #[test]
    fn execute_on_candidate_returns_jump() {
        let mut p = SearchProvider::new(Arc::new(NominatimClient::new()));
        p.candidates.push(SearchResult {
            name: "Tokyo".to_string(),
            location: LonLat {
                lon: 139.69,
                lat: 35.69,
            },
        });
        p.rebuild_items();
        let ctx = ctx();
        match p.execute(0, &ctx) {
            PaletteAction::Run(msgs) => {
                assert_eq!(msgs.len(), 1);
                assert!(matches!(msgs[0], AppMsg::Jump(_)));
            }
            _ => panic!("expected Run([Jump])"),
        }
    }

    #[test]
    fn submit_mode_is_debounced() {
        let p = SearchProvider::new(Arc::new(NominatimClient::new()));
        assert!(matches!(p.submit_mode(), SubmitMode::Debounced(_)));
    }
}
