//! Export plugin — serialises the currently-displayed `MapFrame` as
//! an ANSI-escape text file.
//!
//! Headless: adds a single palette entry ("Export frame as ANSI") and
//! no activation key, no UI. Selecting the entry emits
//! [`AppMsg::ExportFrame`]; [`App::dispatch`](crate::app::App) handles
//! the actual file write because the `MapFrame` + the `ProjectDirs`
//! live there. The plugin itself carries no state.

use crate::plugin_api::prelude::*;

pub fn register(r: &mut Registrar) {
    r.add_run("Export frame as ANSI", "", |_| vec![AppMsg::ExportFrame]);
}
