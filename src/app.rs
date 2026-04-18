use std::io;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyModifiers, MouseButton, MouseEventKind};
use log::{debug, info};
use ratatui::DefaultTerminal;

use crate::config::{Config, KeybindingOverrides};
use crate::core::input::InputHandler;
use crate::core::keymap::KeyMap;
use crate::core::{Action, Core, CoreOptions};
use crate::render::pipeline::RenderPipeline;
use crate::render::thread::{RenderHandle, RenderResult};
use crate::shared::nominatim::NominatimClient;
use crate::styler::Styler;
use crate::ui::UiState;
use crate::ui::layout;
use crate::ui::widget::search::SearchAction;
use crate::ui::widget::wiki::WikiAction;

/// What a key/mouse event just changed. Drives how the main loop
/// reacts: a widget-only change redraws immediately (the map frame
/// is unchanged); a map change only requests a new render — the
/// main loop will redraw when a fresh frame arrives, avoiding a
/// stale-frame draw followed by a second fresh-frame draw.
#[derive(Clone, Copy, PartialEq)]
enum KeyEffect {
    None,
    Widget,
    Map,
}

pub struct App {
    core: Core,
    input: InputHandler,
    render_handle: RenderHandle,
    ui: UiState,
    drag_from: Option<(u16, u16)>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let theme_id = crate::palette::ThemeId::from_name(&config.style);
        let styler = Arc::new(Styler::new(theme_id));

        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let (width, height) = crate::render::canvas_size(cols, rows);

        info!(
            "terminal size: {}x{}, canvas: {}x{}",
            cols, rows, width, height
        );

        let palette = styler.palette();
        let nominatim = Arc::new(NominatimClient::new());
        let mut ui = UiState::new(palette, &config.language, config.wiki_limit, nominatim);
        let pipeline = RenderPipeline::new(
            &config.source,
            config.cache_tiles,
            styler,
            config.language.clone(),
            width,
            height,
        );

        let keymap = build_keymap(&config.keymap);
        let input = InputHandler::new(keymap);
        let core = Core::new(
            CoreOptions {
                initial_lon: config.initial_lon,
                initial_lat: config.initial_lat,
                initial_zoom: config.initial_zoom,
                zoom_step: config.zoom_step,
                max_zoom: config.max_zoom,
            },
            width,
            height,
        );
        let render_handle = RenderHandle::spawn(pipeline);
        ui.help.build(input.keymap());

        App {
            core,
            input,
            render_handle,
            ui,
            drag_from: None,
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

        'main_loop: while self.core.is_running() {
            // 1. Receive completed frames — drain to latest so a burst from
            // the render thread still produces only one redraw.
            while let Ok(RenderResult::Frame(frame)) = self.render_handle.result_rx.try_recv() {
                self.ui.map_frame = Some(frame);
                dirty = true;
            }

            // 2. Poll widgets with background fetches
            if self.ui.search.poll() {
                dirty = true;
            }
            if self.ui.place.poll() {
                dirty = true;
            }
            if self.ui.wiki.poll() {
                dirty = true;
            }

            // 4. Draw (only when something changed)
            if dirty {
                self.draw_terminal(&mut terminal)?;
                dirty = false;
            }

            // 5. Process input events — drain the whole queue in one pass so
            // a burst of key-repeat or mouse-drag events produces a single
            // draw at the top of the next iteration, not one draw per event.
            // First poll blocks up to 4 ms so render-thread frame arrivals
            // (which don't wake the event poll) show up within ~4 ms at
            // worst; subsequent polls use zero timeout to drain the queue.
            let mut poll_timeout = Duration::from_millis(4);
            while event::poll(poll_timeout)? {
                poll_timeout = Duration::from_millis(0);
                match event::read()? {
                    Event::Key(key_event) => {
                        if key_event.modifiers.contains(KeyModifiers::CONTROL)
                            && key_event.code == crossterm::event::KeyCode::Char('c')
                        {
                            info!("Ctrl-C received, quitting");
                            self.core.stop();
                            break 'main_loop;
                        }

                        debug!("key event: {:?}", key_event.code);
                        match self.handle_key(key_event.code, key_event.modifiers) {
                            KeyEffect::None => {}
                            KeyEffect::Widget => dirty = true,
                            KeyEffect::Map => {
                                if self.core.is_running() {
                                    self.request_draw();
                                }
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
                    Event::Mouse(mouse) => match self.handle_mouse(mouse) {
                        KeyEffect::None => {}
                        KeyEffect::Widget => dirty = true,
                        KeyEffect::Map => self.request_draw(),
                    },
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

    fn handle_key(
        &mut self,
        code: crossterm::event::KeyCode,
        modifiers: KeyModifiers,
    ) -> KeyEffect {
        if self.ui.search.is_active() {
            return match self.ui.search.handle_key(code, modifiers) {
                SearchAction::None | SearchAction::Consumed => KeyEffect::Widget,
                SearchAction::Jump(location) => {
                    info!("search: jumping to ({}, {})", location.lat, location.lon);
                    self.core.jump_to(location);
                    KeyEffect::Map
                }
            };
        }

        // Help toggle
        if self.ui.help.is_active() {
            self.ui.help.close();
            return KeyEffect::Widget;
        }

        // Wiki panel navigation
        if self.ui.wiki.is_active() {
            let center = self.core.center();
            match self.ui.wiki.handle_key(code, modifiers, center) {
                WikiAction::None => {}
                WikiAction::Consumed => return KeyEffect::Widget,
                WikiAction::JumpTo(location) => {
                    info!("wiki: jumping to ({}, {})", location.lat, location.lon);
                    self.core.jump_to(location);
                    return KeyEffect::Map;
                }
            }
        }

        let action = self.input.handle_key(code, modifiers);
        match action {
            Action::SearchOpen => {
                self.ui.search.open();
                KeyEffect::Widget
            }
            Action::HelpToggle => {
                self.ui.help.toggle();
                KeyEffect::Widget
            }
            Action::WikiToggle => {
                self.ui.wiki.toggle(self.core.center());
                KeyEffect::Widget
            }
            _ => {
                if self.core.process_action(&action) {
                    KeyEffect::Map
                } else {
                    KeyEffect::None
                }
            }
        }
    }

    fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) -> KeyEffect {
        if self.ui.search.is_active() {
            return KeyEffect::None;
        }

        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let dx = mouse.column as f64 - cols as f64 / 2.0;
        let dy = mouse.row as f64 - rows as f64 / 2.0;

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.drag_from = Some((mouse.column, mouse.row));
                KeyEffect::None
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((prev_x, prev_y)) = self.drag_from {
                    let drag_dx = mouse.column as i16 - prev_x as i16;
                    let drag_dy = mouse.row as i16 - prev_y as i16;
                    self.drag_from = Some((mouse.column, mouse.row));
                    if drag_dx != 0 || drag_dy != 0 {
                        self.core.pan_by_cells(drag_dx, drag_dy);
                        return KeyEffect::Map;
                    }
                }
                KeyEffect::None
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.drag_from = None;
                KeyEffect::None
            }
            MouseEventKind::ScrollUp => {
                self.core.zoom_towards(dx, dy, self.core.zoom_step());
                KeyEffect::Map
            }
            MouseEventKind::ScrollDown => {
                self.core.zoom_towards(dx, dy, -self.core.zoom_step());
                KeyEffect::Map
            }
            _ => KeyEffect::None,
        }
    }

    fn request_draw(&mut self) {
        let state = self.core.render_request();
        self.render_handle.request_draw(state);

        // Notify passive widgets that the map recentered. They decide
        // internally whether to act (e.g., place throttles to 5s).
        // Wiki is intentionally not notified — Google-Maps-style, the
        // article list stays pinned to the query that produced it.
        if !self.ui.search.is_active() {
            self.ui.place.on_map_moved(state.center);
        }
    }

    fn draw_terminal(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        // Coords and scale bar pull directly from the MapFrame inside
        // their overlays, so app.rs no longer pushes derived strings.
        terminal.draw(|f| {
            layout::draw(f, &self.ui);
        })?;
        Ok(())
    }
}

/// Apply `[keymap]` overrides from config onto the default `KeyMap`.
/// Each field in `KeybindingOverrides` names an `Action`; the listed key strings
/// replace any default bindings for that action. Invalid key strings
/// are skipped (logged at warn level by `KeyMap::set_bindings`).
fn build_keymap(overrides: &KeybindingOverrides) -> KeyMap {
    let mut km = KeyMap::default();

    macro_rules! rebind {
        ($field:ident, $action:expr) => {
            if let Some(keys) = &overrides.$field {
                km.set_bindings($action, keys);
            }
        };
    }

    rebind!(pan_left, Action::PanLeft);
    rebind!(pan_right, Action::PanRight);
    rebind!(pan_up, Action::PanUp);
    rebind!(pan_down, Action::PanDown);
    rebind!(pan_left_fast, Action::PanLeftFast);
    rebind!(pan_right_fast, Action::PanRightFast);
    rebind!(pan_up_half, Action::PanUpHalf);
    rebind!(pan_down_half, Action::PanDownHalf);
    rebind!(zoom_in, Action::ZoomIn);
    rebind!(zoom_out, Action::ZoomOut);
    rebind!(zoom_to_world, Action::ZoomToWorld);
    rebind!(reset_position, Action::ResetPosition);
    rebind!(quit, Action::Quit);

    km
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn build_keymap_applies_overrides() {
        let mut overrides = KeybindingOverrides::default();
        overrides.zoom_in = Some(vec!["i".to_string()]);
        overrides.quit = Some(vec!["Q".to_string(), "C-q".to_string()]);

        let km = build_keymap(&overrides);

        assert_eq!(
            km.lookup(KeyCode::Char('i'), KeyModifiers::NONE),
            Some(&Action::ZoomIn)
        );
        assert_eq!(
            km.lookup(KeyCode::Char('Q'), KeyModifiers::NONE),
            Some(&Action::Quit)
        );
        assert_eq!(
            km.lookup(KeyCode::Char('q'), KeyModifiers::CONTROL),
            Some(&Action::Quit)
        );
    }

    #[test]
    fn build_keymap_keeps_unoverridden_defaults() {
        let km = build_keymap(&KeybindingOverrides::default());
        // 'h' is a default PanLeft binding.
        assert_eq!(
            km.lookup(KeyCode::Char('h'), KeyModifiers::NONE),
            Some(&Action::PanLeft)
        );
    }
}
