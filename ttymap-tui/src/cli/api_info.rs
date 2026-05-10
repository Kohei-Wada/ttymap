//! `ttymap api-info` — dump the machine-readable API spec as JSON.
//!
//! Mirrors `nvim --api-info`. External clients (the future
//! `ttymap-mcp` server, editor plugins, out-of-process scripts) call
//! this once at startup to discover the `ttymap.api.*` surface without
//! parsing Rust source or hand-written markdown.
//!
//! Headless: doesn't touch the terminal, doesn't load Lua, doesn't
//! resolve a runtime path — the spec lives entirely in compile-time
//! `&'static` data structures (see [`crate::lua::api::spec`]).

use clap::Args;

use crate::lua::api::spec;

#[derive(Args)]
pub struct ApiInfoArgs {
    /// Pretty-print the JSON output (multi-line, 2-space indent).
    /// Off by default so piping into `jq` stays as fast as possible.
    #[arg(long)]
    pub pretty: bool,
}

pub fn run(args: ApiInfoArgs) -> Result<(), Box<dyn std::error::Error>> {
    let info = spec::all();
    let json = if args.pretty {
        serde_json::to_string_pretty(&info)?
    } else {
        serde_json::to_string(&info)?
    };
    println!("{json}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ttymap api-info` (compact) round-trips through `serde_json` and
    /// the resulting object names every namespace `spec::all()`
    /// declares. Catches both "Serialize derive missing" and "aggregator
    /// silently dropped a namespace" regressions.
    #[test]
    fn run_emits_json_listing_every_aggregated_namespace() {
        let info = spec::all();
        let json = serde_json::to_string(&info).expect("serialise");
        // Each namespace path must appear verbatim in the output.
        for ns in &info.namespaces {
            assert!(
                json.contains(ns.path),
                "namespace path `{}` missing from output: {json}",
                ns.path,
            );
        }
        // Format version is part of the contract — assert it lands at
        // the top level so clients can branch on it.
        assert!(
            json.contains(&format!("\"version\":\"{}\"", spec::SPEC_VERSION)),
            "version field missing or wrong shape: {json}",
        );
    }

    /// Pretty mode produces multi-line output (compact mode is one
    /// line). Quick smoke test — protects against `--pretty` silently
    /// degrading to compact.
    #[test]
    fn pretty_mode_is_multiline() {
        let info = spec::all();
        let pretty = serde_json::to_string_pretty(&info).expect("serialise");
        assert!(pretty.contains('\n'), "pretty output should be multi-line");
    }
}
