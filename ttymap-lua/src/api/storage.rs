//! `ttymap.storage` — per-namespace persistent KV for Lua plugins.
//!
//! Plugins call `ttymap.storage:open("<namespace>")` once and keep
//! the returned `Store` userdata. The `Store` exposes:
//!
//! - `:get(key, default) -> value`
//! - `:set(key, value)`
//! - `:delete(key)`
//!
//! Values are JSON-encoded via [`super::json::lua_to_json`] (so the
//! type rules are exactly the same as `ttymap.json:stringify` —
//! nil / boolean / integer / number / string / table-of-those, no
//! function / userdata / mixed-key tables) and written under
//! `$XDG_DATA_HOME/ttymap/storage/<namespace>/<key>.json`. Writes
//! go through a temp file + rename so a crash mid-write can't
//! corrupt the on-disk copy.
//!
//! `get` is forgiving by design: a missing file or a corrupt /
//! unparseable file both return `default` (with a warning logged
//! for the corrupt case). The "default value" parameter covers the
//! first-run case so a plugin doesn't have to special-case
//! `nil → init`. `delete` is idempotent — removing a missing key
//! is a silent no-op, matching the disposable-handle convention.
//!
//! Namespace and key strings are restricted to `[A-Za-z0-9_-]+`.
//! That keeps us out of path-traversal land (`..`, `/`, `\`,
//! whitespace, control chars all rejected) without needing
//! per-platform filename sanitisation.

use std::path::PathBuf;

use mlua::{UserData, Value};

use super::json::lua_to_json;

/// Top-level `ttymap.storage` userdata. Holds the resolved root
/// directory (`<data_dir>/ttymap/storage`) so every per-call
/// `:open(...)` is a cheap path join — no repeated XDG lookups,
/// and tests can inject a temp dir.
pub(super) struct HostStorage {
    root: PathBuf,
}

impl HostStorage {
    /// Production constructor. `dirs` carries the resolved XDG dirs
    /// via `ttymap-config::AppDirs` (#362); the storage root is
    /// `data_dir().join("storage")`.
    ///
    /// Returns `None` when no per-user data dir is available — the
    /// caller (api/mod.rs) treats that as "skip wiring storage in";
    /// plugins that try to open will get a clear error message.
    pub(super) fn new(dirs: Option<&ttymap_config::AppDirs>) -> Option<Self> {
        let dir = dirs?.data.join("storage");
        Some(Self { root: dir })
    }

    #[cfg(test)]
    fn with_root(root: PathBuf) -> Self {
        Self { root }
    }
}

impl UserData for HostStorage {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("open", |_, this, namespace: String| {
            validate_segment(&namespace, "namespace")?;
            Ok(Store {
                dir: this.root.join(&namespace),
            })
        });
    }
}

/// Per-namespace store handed back from `ttymap.storage:open(name)`.
/// Holds the resolved on-disk dir for the namespace; each call
/// joins the validated key onto it.
pub struct Store {
    dir: PathBuf,
}

impl UserData for Store {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("get", |lua, this, (key, default): (String, Value)| {
            validate_segment(&key, "key")?;
            let path = this.dir.join(format!("{}.json", key));
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // First-run case — caller's default wins. No
                    // log noise; this is the common path on the
                    // very first :get of any plugin.
                    return Ok(default);
                }
                Err(e) => {
                    log::warn!(
                        "lua-host: storage:get({}) read failed: {}",
                        path.display(),
                        e
                    );
                    return Ok(default);
                }
            };
            let json = match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Ok(v) => v,
                Err(e) => {
                    // Corrupt / partially written file — falling
                    // back to the default keeps the plugin alive.
                    // The user can re-save and overwrite.
                    log::warn!(
                        "lua-host: storage:get({}) parse failed: {}; returning default",
                        path.display(),
                        e
                    );
                    return Ok(default);
                }
            };
            super::json::json_to_lua(lua, &json)
        });

        methods.add_method("set", |_, this, (key, value): (String, Value)| {
            validate_segment(&key, "key")?;
            let json = lua_to_json(&value)?;
            let bytes = serde_json::to_vec(&json).map_err(mlua::Error::external)?;
            std::fs::create_dir_all(&this.dir).map_err(mlua::Error::external)?;
            let final_path = this.dir.join(format!("{}.json", key));
            let tmp_path = this.dir.join(format!("{}.json.tmp", key));
            std::fs::write(&tmp_path, &bytes).map_err(mlua::Error::external)?;
            std::fs::rename(&tmp_path, &final_path).map_err(mlua::Error::external)?;
            Ok(())
        });

        methods.add_method("delete", |_, this, key: String| {
            validate_segment(&key, "key")?;
            let path = this.dir.join(format!("{}.json", key));
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(mlua::Error::external(e)),
            }
        });
    }
}

/// Reject anything that isn't a non-empty `[A-Za-z0-9_-]+` token.
/// Conservative on purpose: plugins don't need exotic names, and a
/// strict allowlist sidesteps both path-traversal (`..`, `/`, `\`)
/// and per-platform filename surprises (NUL bytes on Linux, the
/// reserved DOS names on Windows, …).
fn validate_segment(s: &str, what: &str) -> mlua::Result<()> {
    if s.is_empty() {
        return Err(mlua::Error::external(format!(
            "ttymap.storage: {} must be non-empty",
            what
        )));
    }
    let ok = s
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !ok {
        return Err(mlua::Error::external(format!(
            "ttymap.storage: {} must match [A-Za-z0-9_-]+ (got {:?})",
            what, s
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Returns a fresh empty temp dir for the current test. We don't
    /// pull `tempfile` in just for this — the codebase already uses
    /// `std::env::temp_dir()` for similar tests in `lua/mod.rs`.
    fn fresh_root(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("ttymap-storage-test-{label}-{pid}-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn install(lua: &mlua::Lua, root: PathBuf) {
        lua.globals()
            .set(
                "storage",
                lua.create_userdata(HostStorage::with_root(root)).unwrap(),
            )
            .unwrap();
    }

    #[test]
    fn set_then_get_round_trips() {
        let root = fresh_root("rt");
        let lua = mlua::Lua::new();
        install(&lua, root);

        lua.load(
            r#"
            local store = storage:open("rt-plugin")
            store:set("bookmarks", {
                { name = "Tokyo Tower", lon = 139.7454, lat = 35.6586, zoom = 15 },
            })
            local out = store:get("bookmarks", {})
            assert(#out == 1, "len")
            assert(out[1].name == "Tokyo Tower", "name")
            assert(out[1].zoom == 15, "zoom")
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn missing_key_returns_default() {
        let root = fresh_root("default");
        let lua = mlua::Lua::new();
        install(&lua, root);

        lua.load(
            r#"
            local store = storage:open("nope")
            local out = store:get("never-set", { fallback = true })
            assert(out.fallback == true, "default returned")
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn namespaces_are_isolated() {
        let root = fresh_root("isolated");
        let lua = mlua::Lua::new();
        install(&lua, root);

        lua.load(
            r#"
            local a = storage:open("alice")
            local b = storage:open("bob")
            a:set("k", "alice value")
            b:set("k", "bob value")
            assert(a:get("k", "?") == "alice value", "alice owns alice")
            assert(b:get("k", "?") == "bob value", "bob owns bob")
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn corrupt_file_returns_default_and_warns() {
        let root = fresh_root("corrupt");
        // Write garbage directly so :get has something invalid to chew on.
        let dir = root.join("plugin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("k.json"), "{not valid json").unwrap();

        let lua = mlua::Lua::new();
        install(&lua, root);

        lua.load(
            r#"
            local store = storage:open("plugin")
            local out = store:get("k", { fallback = true })
            assert(out.fallback == true, "corrupt → default")
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn delete_is_idempotent() {
        let root = fresh_root("delete");
        let lua = mlua::Lua::new();
        install(&lua, root);

        lua.load(
            r#"
            local store = storage:open("plugin")
            store:set("k", 42)
            store:delete("k")
            store:delete("k")  -- second time must not error
            local out = store:get("k", "missing")
            assert(out == "missing", "after delete the key is gone")
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn invalid_namespace_or_key_rejected() {
        let root = fresh_root("invalid");
        let lua = mlua::Lua::new();
        install(&lua, root);

        // Path-traversal attempt — must error.
        let err = lua
            .load(r#"return storage:open(".."):get("k", nil)"#)
            .exec()
            .unwrap_err();
        assert!(
            err.to_string().contains("namespace must match"),
            "got: {err}"
        );

        // Empty namespace — must error.
        let err = lua.load(r#"return storage:open("")"#).exec().unwrap_err();
        assert!(err.to_string().contains("must be non-empty"), "got: {err}");

        // Slash in key — must error.
        let err = lua
            .load(r#"local s = storage:open("ok"); s:set("a/b", 1)"#)
            .exec()
            .unwrap_err();
        assert!(err.to_string().contains("key must match"), "got: {err}");
    }

    #[test]
    fn set_unsupported_value_errors() {
        // A function-valued field can't be JSON-encoded; the error
        // must propagate so the plugin knows the write didn't happen.
        // (Crucially, the temp file must not be left behind on disk —
        // here we verify only that the call errors; the on-disk
        // absence is a function of std::fs::rename never running.)
        let root = fresh_root("badvalue");
        let lua = mlua::Lua::new();
        install(&lua, root);

        let err = lua
            .load(
                r#"
                local store = storage:open("plugin")
                store:set("k", { fn = function() end })
                "#,
            )
            .exec()
            .unwrap_err();
        assert!(
            err.to_string().contains("unsupported value type"),
            "got: {err}"
        );
    }

    #[test]
    fn atomic_write_does_not_leave_temp_file_on_success() {
        let root = fresh_root("atomic");
        let lua = mlua::Lua::new();
        install(&lua, root.clone());

        lua.load(
            r#"
            local store = storage:open("plugin")
            store:set("k", { ok = true })
            "#,
        )
        .exec()
        .unwrap();

        let dir = root.join("plugin");
        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|r| r.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(entries.contains(&"k.json".to_string()), "final file");
        assert!(
            !entries.iter().any(|n| n.ends_with(".tmp")),
            "no leftover .tmp file: {:?}",
            entries
        );
    }
}
