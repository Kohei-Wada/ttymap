# Contributing

PRs and issues are very welcome — bug reports, feature requests,
plugin ideas, doc fixes, all of it. Don't hesitate to open an
issue even for a half-formed thought.

**Got a fun plugin idea or an API you wish ttymap had?** Send it
over. "It would be cool if ttymap could show X" / "the Lua API
should expose Y" issues are exactly the kind of input that shapes
the roadmap — even if you don't plan to implement it yourself.

> ⚠️ **Heads up:** ttymap is under active development. Breaking
> changes (CLI flags, Lua API, config schema, internal interfaces)
> can land without a deprecation cycle. Sorry in advance — I'll try
> to call them out in commit messages and release notes, but if
> your downstream code or `init.lua` breaks after a `git pull`,
> that's on this project, not on you. File an issue and I'll help
> you migrate.

## How to contribute

- **File an issue** — anything goes. Bug reports, "is this
  intentional?", "would you take a PR for X?", plugin proposals.
- **Write a plugin** — fastest way to extend ttymap without touching
  Rust. Drop a `*.lua` into `~/.config/ttymap/lua/plugin/` to test
  without rebuilding. See
  [docs/lua-architecture.md](docs/lua-architecture.md). Reference
  plugins:
  - Simplest fetch+render: `runtime/lua/plugin/quake.lua`
  - Full panel + selection + modal: `runtime/lua/plugin/wiki/`
  - Debounced palette picker: `runtime/lua/plugin/search/`
- **Add a feature to core** — open an issue first to sanity-check it
  isn't plugin material. Core stays lean (see the Roadmap section
  in [README.md](README.md)).
- **Fix a bug** — PRs welcome. Small fixes don't need a prior issue.

## Dev workflow

```bash
cargo build       # build.rs compiles proto/vector_tile.proto via protox
cargo test
cargo clippy
cargo fmt
```

The pre-commit hook runs tests, clippy, and rustfmt. Don't bypass
it with `--no-verify` — if a hook fails, fix the underlying issue.

For docs: when you change module structure, source-tree layout, or
a documented keybinding/CLI flag, grep the corresponding docs
(`README.md`, `CLAUDE.md`, `docs/`) for stale references and
update them in the same PR. Docs are part of "done".

## License

By contributing, you agree your contributions will be dual-licensed
under Apache-2.0 and MIT, matching the rest of the project (see
[LICENSE-APACHE](LICENSE-APACHE) / [LICENSE-MIT](LICENSE-MIT)).
