//! UI key router — focus-aware translation from raw key events into
//! `Option<AppCommand>`. Sits between the input layer (which only adapts
//! crossterm events) and `app_command::dispatch` (which only routes intent
//! to a single domain method per arm).
//!
//! Routing is expressed as an ordered table of [`Stage`]s ([`ROUTING`]).
//! Each stage produces one of `{Consumed, Run(cmd), Pass}`; the first
//! non-`Pass` stage wins. To add or reorder a stage: edit [`ROUTING`]
//! and [`apply_stage`].
//!
//! Focus writes and state dispatch never happen here — they're all in
//! `app_command::dispatch` / `UiState`. This layer only *reads* focus
//! (indirectly, via `UiState::deliver_key`) and produces `AppCommand`
//! values.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::app_command::{AppCommand, KeyDelivery};
use crate::geo::LonLat;
use crate::keymap::KeyMap;
use crate::map::Action;
use crate::ui::UiState;

/// One priority tier in the routing pipeline. The *order* of tiers is
/// the sole responsibility of [`ROUTING`]; each variant's *behaviour*
/// lives in [`apply_stage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Hand the key to the currently-focused surface (palette / plugin).
    /// That surface may consume the key, emit an `AppCommand`, or pass.
    FocusDelivery,
    /// Tab / Shift-Tab → cycle focus across visible plugins.
    CycleFocus,
    /// `:` → open the command palette.
    OpenPalette,
    /// Match the key against every plugin's activation binding.
    PluginActivation,
    /// Resolve against `KeyMap` (pan / zoom / reset / …).
    KeymapFallback,
}

/// The routing pipeline, in priority order. First non-`Pass` stage wins.
const ROUTING: &[Stage] = &[
    Stage::FocusDelivery,
    Stage::CycleFocus,
    Stage::OpenPalette,
    Stage::PluginActivation,
    Stage::KeymapFallback,
];

enum StageOutcome {
    Consumed,
    Run(AppCommand),
    Pass,
}

pub struct KeyRouter {
    keymap: KeyMap,
    /// First-`g`-of-`gg` flag. Multi-key sequences are a router
    /// concern — the keymap itself is a pure lookup table.
    pending_g: bool,
}

impl KeyRouter {
    pub fn new(keymap: KeyMap) -> Self {
        Self {
            keymap,
            pending_g: false,
        }
    }

    /// Expose the keymap so `app.rs` can thread it into the
    /// `DispatchCtx` it builds for `app_command::dispatch`.
    pub fn keymap(&self) -> &KeyMap {
        &self.keymap
    }

    /// Advance the `gg` sequence state machine and resolve via the
    /// keymap. Returns the `AppCommand` to dispatch, or `None` for a
    /// no-op (mid-sequence or unbound key).
    fn resolve_with_sequence(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<AppCommand> {
        if code == KeyCode::Char('g') && modifiers == KeyModifiers::NONE {
            if self.pending_g {
                self.pending_g = false;
                return Some(AppCommand::Map(Action::ZoomToWorld));
            }
            self.pending_g = true;
            return None;
        }
        self.pending_g = false;
        self.keymap.resolve(code, modifiers)
    }

    /// Translate a raw key event into an optional `AppCommand`. Side
    /// effects are limited to focused-surface delivery (palette filter
    /// edit, plugin state update) and focus auto-release — both
    /// performed inside `UiState::deliver_key`. The caller runs
    /// `app_command::dispatch` on the returned `AppCommand`.
    pub fn route_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ui: &mut UiState,
        center: LonLat,
    ) -> Option<AppCommand> {
        // Always advance the gg-sequence state machine, even if a
        // higher-priority stage consumes this key — otherwise Tab or
        // `:` between two `g` presses wouldn't break the sequence.
        let fallback_cmd = self.resolve_with_sequence(code, modifiers);

        for stage in ROUTING {
            match apply_stage(*stage, code, modifiers, ui, center, &fallback_cmd) {
                StageOutcome::Consumed => return None,
                StageOutcome::Run(cmd) => return Some(cmd),
                StageOutcome::Pass => continue,
            }
        }
        None
    }
}

/// Evaluate one routing stage. Each arm is small enough to read at a
/// glance; the full pipeline order is in [`ROUTING`].
fn apply_stage(
    stage: Stage,
    code: KeyCode,
    modifiers: KeyModifiers,
    ui: &mut UiState,
    center: LonLat,
    fallback: &Option<AppCommand>,
) -> StageOutcome {
    match stage {
        Stage::FocusDelivery => match ui.deliver_key(code, modifiers, center) {
            KeyDelivery::Consumed => StageOutcome::Consumed,
            KeyDelivery::Run(cmd) => StageOutcome::Run(cmd),
            KeyDelivery::Passthrough => StageOutcome::Pass,
        },
        Stage::CycleFocus => {
            let forward = code == KeyCode::Tab && modifiers == KeyModifiers::NONE;
            let backward = code == KeyCode::BackTab
                || (code == KeyCode::Tab && modifiers.contains(KeyModifiers::SHIFT));
            if forward || backward {
                StageOutcome::Run(AppCommand::CycleFocus(forward))
            } else {
                StageOutcome::Pass
            }
        }
        Stage::OpenPalette => {
            if code == KeyCode::Char(':') && modifiers == KeyModifiers::NONE {
                StageOutcome::Run(AppCommand::OpenPalette)
            } else {
                StageOutcome::Pass
            }
        }
        Stage::PluginActivation => match ui.widgets.activation_tag(code, modifiers) {
            Some(tag) => StageOutcome::Run(AppCommand::ActivatePlugin(tag.to_string())),
            None => StageOutcome::Pass,
        },
        Stage::KeymapFallback => match fallback {
            Some(cmd) => StageOutcome::Run(cmd.clone()),
            None => StageOutcome::Pass,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: KeyModifiers = KeyModifiers::NONE;

    fn map(action: Action) -> AppCommand {
        AppCommand::Map(action)
    }

    #[test]
    fn gg_produces_zoom_to_world_on_second_g() {
        let mut r = KeyRouter::new(KeyMap::default());
        assert_eq!(r.resolve_with_sequence(KeyCode::Char('g'), NONE), None);
        assert_eq!(
            r.resolve_with_sequence(KeyCode::Char('g'), NONE),
            Some(map(Action::ZoomToWorld))
        );
    }

    #[test]
    fn gg_sequence_broken_by_other_key() {
        let mut r = KeyRouter::new(KeyMap::default());
        r.resolve_with_sequence(KeyCode::Char('g'), NONE);
        r.resolve_with_sequence(KeyCode::Char('h'), NONE); // breaks
        assert_eq!(r.resolve_with_sequence(KeyCode::Char('g'), NONE), None);
    }

    /// The routing table is the public contract of this module;
    /// changing its order is a behavioural change. Lock it in.
    #[test]
    fn routing_table_order_is_stable() {
        assert_eq!(
            ROUTING,
            &[
                Stage::FocusDelivery,
                Stage::CycleFocus,
                Stage::OpenPalette,
                Stage::PluginActivation,
                Stage::KeymapFallback,
            ],
        );
    }
}
