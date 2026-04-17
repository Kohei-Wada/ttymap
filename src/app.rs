use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Throttle {
    last: Option<Instant>,
    interval: Duration,
}

impl Throttle {
    fn ready(interval: Duration) -> Self {
        Self {
            last: None,
            interval,
        }
    }

    fn with_cooldown(interval: Duration) -> Self {
        Self {
            last: Some(Instant::now()),
            interval,
        }
    }

    fn check(&mut self) -> bool {
        let ready = self.last.is_none_or(|t| t.elapsed() >= self.interval);
        if ready {
            self.last = Some(Instant::now());
        }
        ready
    }
}

use crossterm::event::{self, Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use log::{debug, info};
use ratatui::DefaultTerminal;

use crate::core::input::InputHandler;
use crate::core::{Action, Config, Core};
use crate::geocode::{GeoResponse, Geocoder};
use crate::nominatim::SearchResult;
use crate::palette;
use crate::render::pipeline::RenderPipeline;
use crate::render::thread::{RenderHandle, RenderResult};
use crate::styler::{StylePreset, Styler};
use crate::ui::UiState;
use crate::ui::layout;
use crate::ui::widget::search::SearchAction;
use crate::ui::widget::wiki::WikiAction;
use crate::wikipedia::{WikiArticle, WikipediaClient};

pub struct App {
    core: Core,
    input: InputHandler,
    render_handle: RenderHandle,
    geocoder: Geocoder,
    ui: UiState,
    last_search_results: Vec<SearchResult>,
    reverse_throttle: Throttle,
    drag_from: Option<(u16, u16)>,
    wiki_rx: std::sync::mpsc::Receiver<Vec<WikiArticle>>,
    wiki_tx: std::sync::mpsc::Sender<Vec<WikiArticle>>,
    wiki_language: String,
    wiki_limit: u32,
    wiki_throttle: Throttle,
}

impl App {
    pub fn new(mut config: Config) -> Self {
        let styler = Arc::new(Styler::new(config.style_preset));
        let language = config.language.clone();
        let wiki_language = language.clone();
        let wiki_limit = config.wiki_limit;

        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let (width, height) = crate::render::canvas_size(cols, rows);

        info!(
            "terminal size: {}x{}, canvas: {}x{}",
            cols, rows, width, height
        );

        let pipeline = RenderPipeline::new(
            &config.source,
            config.cache_tiles,
            styler,
            language,
            width,
            height,
        );
        let p = match config.style_preset {
            StylePreset::Dark => &palette::DARK,
            StylePreset::Bright => &palette::BRIGHT,
        };
        let mut ui = UiState::new(p);

        let keymap = std::mem::take(&mut config.keymap);
        let input = InputHandler::new(keymap);
        let core = Core::new(config, width, height);
        let render_handle = RenderHandle::spawn(pipeline);
        ui.help.build(input.keymap());

        let (wiki_tx, wiki_rx) = std::sync::mpsc::channel();

        App {
            core,
            input,
            render_handle,
            geocoder: Geocoder::new(),
            ui,
            last_search_results: Vec::new(),
            reverse_throttle: Throttle::ready(Duration::from_secs(5)),
            drag_from: None,
            wiki_rx,
            wiki_tx,
            wiki_language,
            wiki_limit,
            wiki_throttle: Throttle::with_cooldown(Duration::from_secs(2)),
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        let mut terminal = ratatui::init();
        crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture)?;

        info!("event loop started");
        self.request_draw();

        // Only redraw the terminal when something has changed. Without this
        // the main loop redraws on every poll tick (~60 Hz) even when the
        // app is idle, which makes the CPU fan spin up.
        let mut dirty = true;

        while self.core.is_running() {
            // 1. Receive completed frames
            while let Ok(RenderResult::Frame(frame)) = self.render_handle.result_rx.try_recv() {
                debug!("frame received");
                self.ui.map_frame = Some(frame);
                dirty = true;
            }

            // 2. Poll geocode results
            if let Some(response) = self.geocoder.poll() {
                match response {
                    GeoResponse::Search(results) => {
                        if results.is_empty() {
                            info!("geocode: no results");
                        } else {
                            info!("geocode: {} results", results.len());
                            self.last_search_results = results.clone();
                            self.ui.search.set_candidates(results);
                        }
                    }
                    GeoResponse::Reverse(place) => {
                        if let Some(place_info) = place {
                            debug!("reverse: {}", place_info.display_name);
                            let name = match (&place_info.city, &place_info.country) {
                                (Some(city), Some(country)) => format!("{}, {}", city, country),
                                (None, Some(country)) => country.clone(),
                                (Some(city), None) => city.clone(),
                                (None, None) => place_info.display_name.clone(),
                            };
                            self.ui.info.set_place(Some(name));
                        }
                    }
                }
                dirty = true;
            }

            // 3. Poll wiki results
            if let Ok(articles) = self.wiki_rx.try_recv() {
                debug!("wiki: received {} articles", articles.len());
                self.ui.wiki.set_articles(articles);
                dirty = true;
            }

            // 4. Draw (only when something changed)
            if dirty {
                self.draw_terminal(&mut terminal)?;
                dirty = false;
            }

            // 5. Process input events
            if event::poll(Duration::from_millis(16))? {
                match event::read()? {
                    Event::Key(key_event) => {
                        if key_event.modifiers.contains(KeyModifiers::CONTROL)
                            && key_event.code == crossterm::event::KeyCode::Char('c')
                        {
                            info!("Ctrl-C received, quitting");
                            self.core.stop();
                            break;
                        }

                        debug!("key event: {:?}", key_event.code);
                        let should_redraw = self.handle_key(key_event.code, key_event.modifiers);
                        if should_redraw {
                            dirty = true;
                            if self.core.is_running() {
                                self.request_draw();
                            }
                        }
                    }
                    Event::Resize(cols, rows) => {
                        info!("resize: {}x{}", cols, rows);
                        self.core.resize(cols, rows);
                        self.render_handle
                            .request_resize(self.core.width(), self.core.height());
                        self.request_draw();
                        dirty = true;
                    }
                    Event::Mouse(mouse) if self.handle_mouse(mouse) => {
                        self.request_draw();
                        dirty = true;
                    }
                    _ => {}
                }
            }
        }

        info!("event loop ended, shutting down render thread");
        self.render_handle.shutdown();
        crossterm::execute!(io::stdout(), crossterm::event::DisableMouseCapture)?;
        ratatui::restore();
        info!("terminal restored, exiting");

        Ok(())
    }

    fn handle_key(&mut self, code: crossterm::event::KeyCode, modifiers: KeyModifiers) -> bool {
        if self.ui.search.is_active() {
            match self.ui.search.handle_key(code, modifiers) {
                SearchAction::Submit(query) => {
                    info!("geocode: searching '{}'", query);
                    self.geocoder.search(&query);
                }
                SearchAction::Select(idx) => {
                    if let Some(result) = self.last_search_results.get(idx) {
                        info!(
                            "geocode: jumping to '{}' ({}, {})",
                            result.name, result.location.lat, result.location.lon
                        );
                        self.core.jump_to(result.location);
                        self.request_draw();
                    }
                }
                SearchAction::Cancel | SearchAction::None => {}
            }
            return true;
        }

        // Help toggle
        if self.ui.help.is_active() {
            self.ui.help.close();
            return true;
        }

        // Wiki panel navigation
        if self.ui.wiki.is_active() {
            match self.ui.wiki.handle_key(code, modifiers) {
                WikiAction::JumpTo(location) => {
                    info!("wiki: jumping to ({}, {})", location.lat, location.lon);
                    self.core.jump_to(location);
                    self.request_draw();
                    return true;
                }
                WikiAction::None => {}
            }
            // Manual refresh at the current map center.
            if code == KeyCode::Char('r') {
                self.fetch_wiki();
                return true;
            }
            // Consume navigation / widget-control keys so they don't fall
            // through to the global keymap. Enter/Esc/Backspace may have
            // toggled the detail view inside the widget — they need to
            // return `true` here so the main loop redraws.
            let ctrl = modifiers.contains(KeyModifiers::CONTROL);
            if matches!(
                code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Enter
                    | KeyCode::Esc
                    | KeyCode::Backspace
            ) || (ctrl
                && matches!(
                    code,
                    KeyCode::Char('j')
                        | KeyCode::Char('k')
                        | KeyCode::Char('n')
                        | KeyCode::Char('p')
                ))
            {
                return true;
            }
        }

        let action = self.input.handle_key(code, modifiers);
        match action {
            Action::SearchOpen => {
                self.ui.search.open();
                true
            }
            Action::HelpToggle => {
                self.ui.help.toggle();
                true
            }
            Action::WikiToggle => {
                self.ui.wiki.toggle();
                if self.ui.wiki.is_active() {
                    self.fetch_wiki();
                }
                true
            }
            _ => self.core.process_action(&action),
        }
    }

    fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) -> bool {
        if self.ui.search.is_active() {
            return false;
        }

        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let dx = mouse.column as f64 - cols as f64 / 2.0;
        let dy = mouse.row as f64 - rows as f64 / 2.0;

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.drag_from = Some((mouse.column, mouse.row));
                false
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((prev_x, prev_y)) = self.drag_from {
                    let drag_dx = mouse.column as i16 - prev_x as i16;
                    let drag_dy = mouse.row as i16 - prev_y as i16;
                    self.drag_from = Some((mouse.column, mouse.row));
                    if drag_dx != 0 || drag_dy != 0 {
                        self.core.pan_by_cells(drag_dx, drag_dy);
                        return true;
                    }
                }
                false
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.drag_from = None;
                false
            }
            MouseEventKind::ScrollUp => {
                self.core.zoom_towards(dx, dy, self.core.zoom_step());
                true
            }
            MouseEventKind::ScrollDown => {
                self.core.zoom_towards(dx, dy, -self.core.zoom_step());
                true
            }
            _ => false,
        }
    }

    fn fetch_wiki(&mut self) {
        if !self.wiki_throttle.check() {
            return;
        }
        let req = self.core.render_request();
        let tx = self.wiki_tx.clone();
        let lang = self.wiki_language.clone();
        let limit = self.wiki_limit;
        std::thread::spawn(move || {
            if let Some(client) = WikipediaClient::new(&lang) {
                let articles = client.geosearch(req.center.lat, req.center.lon, limit);
                let _ = tx.send(articles);
            }
        });
    }

    fn request_draw(&mut self) {
        let state = self.core.render_request();
        self.render_handle.request_draw(state);

        // Debounced reverse geocoding (5 seconds, skip during search)
        if !self.ui.search.is_active() && self.reverse_throttle.check() {
            let req = self.core.render_request();
            self.geocoder.reverse(req.center);
        }

        // NOTE: wiki is intentionally NOT refreshed here. Google-Maps-style,
        // the list stays pinned to the query that produced it; use `r` to
        // re-fetch at the current map center.
    }

    fn draw_terminal(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        self.ui.info.set_coords(self.core.status_bar());

        let req = self.core.render_request();
        let (cols, _) = crossterm::terminal::size().unwrap_or((80, 24));
        let (label, width) = crate::geo::scale_bar(req.center.lat, req.zoom, cols);
        self.ui.info.set_scale(label, width);

        terminal.draw(|f| {
            layout::draw(f, &self.ui);
        })?;
        Ok(())
    }
}
