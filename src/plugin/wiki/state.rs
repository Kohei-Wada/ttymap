//! Wiki plugin state — owns the polled article feed, the current
//! selection, and the optional detail-view snapshot. Shared between
//! the [`crate::plugin::wiki::component::WikiComponent`] handle and
//! the [`crate::plugin::wiki::panel::render_panel`] read-side.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use log::debug;

use crate::plugin_api::prelude::*;

use super::wikipedia::{WikiArticle, WikipediaClient};

/// Shared state for the wiki subsystem. Lives behind an
/// `Rc<RefCell<_>>` so every [`crate::plugin::wiki::component::WikiComponent`]
/// push (and any future second view) shares one source of truth.
pub struct WikiState {
    pub(in crate::plugin::wiki) articles: Vec<WikiArticle>,
    pub(in crate::plugin::wiki) selected: usize,
    /// Snapshot of the article being viewed in detail mode. A copy
    /// (not an index) so it survives even if the candidate list is
    /// refreshed (e.g. after the map panned and new nearby articles
    /// loaded).
    pub(in crate::plugin::wiki) detail: Option<WikiArticle>,
    /// User-supplied panel placement override; applied at render
    /// time. Set from `[wiki]` config section.
    pub(in crate::plugin::wiki) layout: LayoutConfig,
    client: Arc<WikipediaClient>,
    limit: u32,
    feed: PolledFeed<Vec<WikiArticle>>,
}

impl WikiState {
    pub fn new(language: &str, limit: u32, layout: LayoutConfig) -> Self {
        Self {
            articles: Vec::new(),
            selected: 0,
            detail: None,
            layout,
            client: Arc::new(WikipediaClient::new(language)),
            limit,
            feed: PolledFeed::with_cooldown(Duration::from_secs(2)),
        }
    }

    pub fn is_detail_open(&self) -> bool {
        self.detail.is_some()
    }

    pub(super) fn refresh(&mut self, center: LonLat) {
        let client = self.client.clone();
        let limit = self.limit;
        self.feed
            .refresh(move || client.geosearch(center.lat, center.lon, limit));
    }

    /// Merge fresh fetch results into the existing list — preserve
    /// articles still present, append new ones, clamp `selected` if
    /// the list shrank. Avoids disturbing the user's selection on a
    /// re-fetch.
    fn set_articles(&mut self, new_articles: Vec<WikiArticle>) {
        let new_titles: HashSet<String> = new_articles.iter().map(|a| a.title.clone()).collect();
        self.articles.retain(|a| new_titles.contains(&a.title));

        let existing_titles: HashSet<String> =
            self.articles.iter().map(|a| a.title.clone()).collect();
        for article in new_articles {
            if !existing_titles.contains(&article.title) {
                self.articles.push(article);
            }
        }

        if self.selected >= self.articles.len() {
            self.selected = self.articles.len().saturating_sub(1);
        }
    }

    /// Advance async fetch, merging new results when available.
    pub fn poll(&mut self) -> bool {
        if let Some(articles) = self.feed.poll() {
            debug!("wiki: received {} articles", articles.len());
            self.set_articles(articles);
            true
        } else {
            false
        }
    }
}

pub type WikiHandle = Rc<RefCell<WikiState>>;
