//! Machine-readable spec types for the `ttymap.api.*` surface.
//!
//! Each `ttymap-tui/src/lua/api/<ns>.rs` exposes a `pub fn spec() ->
//! NamespaceSpec` that describes its methods — name, params, return
//! shape, doc summary. The CLI subcommand `ttymap api-info` (issue
//! #300) aggregates these and prints JSON; external clients
//! (`ttymap-mcp`, editor plugins) consume that JSON to discover the
//! surface without parsing Rust source or hand-written markdown.
//!
//! Inspired by `nvim --api-info` — nvim ships per-method metadata
//! (params, return type, since-version) over its RPC layer, which is
//! how pynvim and other clients stay in sync without code-generation
//! per release.
//!
//! Everything is `&'static` so each `spec()` is effectively a const
//! lookup. `ty` strings use Lua's base type names (`number`, `string`,
//! `boolean`, `table`, `function`, `nil`, `userdata`); add `?` for
//! optional (`number?`); use a tuple form for multiple returns
//! (`(number, number)`).

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct NamespaceSpec {
    /// Lua-side dotted path the namespace surfaces under
    /// (e.g. `"ttymap.map"`).
    pub path: &'static str,
    /// Methods reachable on the namespace. Order is the source-of-truth
    /// definition order — preserved for stable doc generation.
    pub methods: &'static [MethodSpec],
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct MethodSpec {
    pub name: &'static str,
    pub params: &'static [ParamSpec],
    /// Lua type name of the return value. Use `"nil"` for fire-and-forget
    /// methods; `"(t1, t2)"` for multi-return; suffix `?` for optional.
    pub returns: &'static str,
    /// One-paragraph summary. Keep it focused on **what** the method
    /// does and any non-obvious semantics; longer prose belongs in
    /// `docs/lua-architecture.md`.
    pub doc: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ParamSpec {
    pub name: &'static str,
    pub ty: &'static str,
}

/// Top-level dump shape the `ttymap api-info` subcommand serialises.
/// `version` tracks the spec format itself, **not** the crate version;
/// bump it when the JSON shape (not its contents) changes so external
/// clients can branch on incompatibility.
#[derive(Debug, Clone, Serialize)]
pub struct ApiInfo {
    pub version: &'static str,
    pub namespaces: Vec<NamespaceSpec>,
}

/// Format version of the JSON dump itself. Bump on shape changes —
/// adding methods to a namespace doesn't count.
pub const SPEC_VERSION: &str = "0.1";

/// Aggregate every namespace that has a `spec()`. Listed by hand so
/// adding a new spec is a one-line edit here; rolling out spec coverage
/// to the rest of the namespaces is exactly that — append a line.
pub fn all() -> ApiInfo {
    ApiInfo {
        version: SPEC_VERSION,
        namespaces: vec![super::map::spec()],
    }
}
