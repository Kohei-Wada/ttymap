//! `ttymap.json` — JSON parsing surface for Lua plugins.
//!
//! Two methods on a single userdata:
//! - `ttymap.json:parse(s) -> value | nil` — JSON text → Lua value.
//!   Objects become string-keyed tables, arrays become 1-indexed
//!   tables, `null` becomes `nil`. Parse errors are swallowed
//!   (warning logged) so a flaky upstream doesn't take a plugin down.
//! - `ttymap.json:stringify(value) -> string` — Lua value → JSON
//!   text. Type-strict: function / userdata / thread are rejected
//!   with an error so a silent drop can't corrupt persisted state.
//!   Mixed-key tables (integer + string keys) are rejected for the
//!   same reason. NaN / Infinity become JSON `null`.
//!
//! `lua_to_json` is the bidirectional sibling of `json_to_lua` and
//! is shared with `ttymap.storage` (which JSON-encodes values
//! before atomic-writing to disk).
//!
//! The two methods round-trip: `parse(stringify(x))` preserves
//! type for the supported value set (nil / boolean / integer /
//! number / string / array table / object table).

use mlua::UserData;

pub struct HostJson;

impl UserData for HostJson {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method(
            "parse",
            |lua, _this, source: String| match serde_json::from_str::<serde_json::Value>(&source) {
                Ok(v) => json_to_lua(lua, &v).map(Some),
                Err(e) => {
                    log::warn!("lua-host: json:parse failed: {}", e);
                    Ok(None)
                }
            },
        );

        methods.add_method("stringify", |_lua, _this, value: mlua::Value| {
            let json = lua_to_json(&value)?;
            serde_json::to_string(&json).map_err(mlua::Error::external)
        });
    }
}

/// Recursive translation of a `serde_json::Value` into a
/// `mlua::Value`. Objects map to string-keyed tables, arrays to
/// 1-indexed tables (Lua convention), null to nil, integers to
/// `Integer` when they fit and `Number` otherwise.
///
/// `pub(super)` so `ttymap.storage` (sibling) can reuse the same
/// decoder when reading a JSON-on-disk value back into Lua.
pub(super) fn json_to_lua(lua: &mlua::Lua, value: &serde_json::Value) -> mlua::Result<mlua::Value> {
    match value {
        serde_json::Value::Null => Ok(mlua::Value::Nil),
        serde_json::Value::Bool(b) => Ok(mlua::Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(mlua::Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(mlua::Value::Number(f))
            } else {
                // Numbers that fit neither i64 nor f64 are
                // exotic (large unsigned). Surface as nil rather
                // than panic; plugins can do their own handling.
                Ok(mlua::Value::Nil)
            }
        }
        serde_json::Value::String(s) => Ok(mlua::Value::String(lua.create_string(s)?)),
        serde_json::Value::Array(items) => {
            let table = lua.create_table()?;
            // Lua arrays are 1-indexed.
            for (i, item) in items.iter().enumerate() {
                table.set(i + 1, json_to_lua(lua, item)?)?;
            }
            Ok(mlua::Value::Table(table))
        }
        serde_json::Value::Object(map) => {
            let table = lua.create_table()?;
            for (k, v) in map {
                table.set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            Ok(mlua::Value::Table(table))
        }
    }
}

/// Translate a `mlua::Value` into a `serde_json::Value`.
///
/// Type rules (kept tight to avoid silent corruption of persisted
/// state):
/// - `nil` → `null`
/// - `boolean` → `bool`
/// - `integer` → integer JSON number
/// - `number` → `f64` JSON number; `NaN` / `±Infinity` → `null`
///   (JSON has no way to express them — we drop rather than fail
///   so that a single bad cell in a larger table doesn't sink the
///   whole encode)
/// - `string` → JSON string (UTF-8; non-UTF-8 bytes are rejected)
/// - `table` → array if every key is `1..=#t` consecutive integers,
///   otherwise object (string keys only). Mixed integer + string
///   keys are rejected with an error.
/// - `function` / `userdata` / `thread` / `lightuserdata` are
///   rejected with an error.
pub(crate) fn lua_to_json(value: &mlua::Value) -> mlua::Result<serde_json::Value> {
    match value {
        mlua::Value::Nil => Ok(serde_json::Value::Null),
        mlua::Value::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        mlua::Value::Integer(i) => Ok(serde_json::Value::from(*i)),
        mlua::Value::Number(n) => {
            if n.is_finite() {
                Ok(serde_json::Value::from(*n))
            } else {
                Ok(serde_json::Value::Null)
            }
        }
        mlua::Value::String(s) => Ok(serde_json::Value::String(s.to_str()?.to_owned())),
        mlua::Value::Table(t) => table_to_json(t),
        other => Err(mlua::Error::external(format!(
            "ttymap.json:stringify: unsupported value type {}",
            other.type_name()
        ))),
    }
}

/// Decide whether a Lua table is a JSON array or object and encode
/// accordingly. The decision is purely structural: every key is a
/// positive integer covering `1..=n` exactly → array; every key is
/// a string → object; anything else → error.
fn table_to_json(table: &mlua::Table) -> mlua::Result<serde_json::Value> {
    // First sweep: peek at every key to classify the table. We don't
    // start emitting until we know which container to build, because
    // a half-encoded array can't be promoted to an object.
    let mut max_int: i64 = 0;
    let mut int_count: usize = 0;
    let mut has_string_key = false;
    for pair in table.clone().pairs::<mlua::Value, mlua::Value>() {
        let (k, _) = pair?;
        match k {
            mlua::Value::Integer(i) if i >= 1 => {
                int_count += 1;
                if i > max_int {
                    max_int = i;
                }
            }
            mlua::Value::String(_) => has_string_key = true,
            mlua::Value::Integer(_) | mlua::Value::Number(_) => {
                return Err(mlua::Error::external(
                    "ttymap.json:stringify: table has non-positive numeric key",
                ));
            }
            other => {
                return Err(mlua::Error::external(format!(
                    "ttymap.json:stringify: table has unsupported key type {}",
                    other.type_name()
                )));
            }
        }
    }

    let is_pure_array = int_count > 0 && !has_string_key && max_int as usize == int_count;
    let is_pure_object = has_string_key && int_count == 0;
    let is_empty = int_count == 0 && !has_string_key;

    if is_empty {
        // Empty Lua table → empty JSON object. Lua plugins overwhelm-
        // ingly use `{}` to mean "object I haven't put anything in
        // yet" (see config tables, opts arguments). The asymmetric
        // rare case — empty array — can be expressed by stringifying
        // a populated array; round-trip preserves it.
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    }

    if is_pure_array {
        let mut items = Vec::with_capacity(int_count);
        for i in 1..=max_int {
            let v: mlua::Value = table.get(i)?;
            items.push(lua_to_json(&v)?);
        }
        return Ok(serde_json::Value::Array(items));
    }

    if is_pure_object {
        let mut map = serde_json::Map::with_capacity(int_count + (has_string_key as usize));
        for pair in table.clone().pairs::<mlua::String, mlua::Value>() {
            let (k, v) = pair?;
            map.insert(k.to_str()?.to_owned(), lua_to_json(&v)?);
        }
        return Ok(serde_json::Value::Object(map));
    }

    Err(mlua::Error::external(
        "ttymap.json:stringify: table mixes integer and string keys (cannot encode as JSON)",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> mlua::Lua {
        let lua = mlua::Lua::new();
        lua.globals()
            .set("json", lua.create_userdata(HostJson).unwrap())
            .unwrap();
        lua
    }

    #[test]
    fn stringify_primitives() {
        let lua = host();
        let cases = [
            ("nil", "null"),
            ("true", "true"),
            ("false", "false"),
            ("0", "0"),
            ("42", "42"),
            ("-7", "-7"),
            ("1.5", "1.5"),
            ("\"hi\"", "\"hi\""),
            ("\"a\\nb\"", "\"a\\nb\""),
        ];
        for (input, want) in cases {
            let got: String = lua
                .load(format!("return json:stringify({input})"))
                .eval()
                .unwrap_or_else(|e| panic!("stringify({input}): {e}"));
            assert_eq!(got, want, "stringify({input})");
        }
    }

    #[test]
    fn stringify_array_table() {
        let lua = host();
        let got: String = lua.load("return json:stringify({1, 2, 3})").eval().unwrap();
        assert_eq!(got, "[1,2,3]");
    }

    #[test]
    fn stringify_object_table() {
        let lua = host();
        // Order is not guaranteed across runs, so parse round-trip
        // and assert key set instead of literal string match.
        let got: String = lua
            .load(r#"return json:stringify({name = "Tokyo", lon = 139.7, lat = 35.6})"#)
            .eval()
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&got).unwrap();
        let obj = v.as_object().expect("object");
        assert_eq!(obj["name"], "Tokyo");
        assert_eq!(obj["lon"], 139.7);
        assert_eq!(obj["lat"], 35.6);
    }

    #[test]
    fn stringify_nested() {
        let lua = host();
        let got: String = lua
            .load(r#"return json:stringify({{name = "A", lon = 1}, {name = "B", lon = 2}})"#)
            .eval()
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&got).unwrap();
        let arr = v.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], "A");
        assert_eq!(arr[1]["lon"], 2);
    }

    #[test]
    fn stringify_empty_table_is_object() {
        // Lua's overwhelming convention for `{}` is "empty object".
        // Document the choice via test.
        let lua = host();
        let got: String = lua.load("return json:stringify({})").eval().unwrap();
        assert_eq!(got, "{}");
    }

    #[test]
    fn stringify_mixed_key_table_errors() {
        let lua = host();
        let err = lua
            .load("return json:stringify({1, 2, name = 'oops'})")
            .eval::<String>()
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("mixes integer and string keys"),
            "expected mixed-key error, got: {msg}"
        );
    }

    #[test]
    fn stringify_unsupported_function_errors() {
        let lua = host();
        let err = lua
            .load("return json:stringify(function() end)")
            .eval::<String>()
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unsupported value type"),
            "expected unsupported-value error, got: {msg}"
        );
    }

    #[test]
    fn stringify_nan_and_infinity_become_null() {
        let lua = host();
        let nan: String = lua.load("return json:stringify(0/0)").eval().unwrap();
        let inf: String = lua.load("return json:stringify(1/0)").eval().unwrap();
        let neg_inf: String = lua.load("return json:stringify(-1/0)").eval().unwrap();
        assert_eq!(nan, "null");
        assert_eq!(inf, "null");
        assert_eq!(neg_inf, "null");
    }

    #[test]
    fn stringify_sparse_array_errors() {
        // {[1]=1, [3]=3} has gap at 2 — not a valid JSON array. Reject
        // rather than silently fill with null.
        let lua = host();
        let err = lua
            .load("return json:stringify({[1]=1, [3]=3})")
            .eval::<String>()
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("mixes integer and string keys") || msg.contains("non-positive"),
            "expected sparse-array rejection, got: {msg}"
        );
    }

    #[test]
    fn round_trip_preserves_types() {
        let lua = host();
        // Each script asserts the round-trip on its own value so we
        // exercise integer / number / bool / string / nested object.
        lua.load(
            r#"
            local input = {
                name = "Tokyo Tower",
                lon = 139.7454,
                lat = 35.6586,
                zoom = 15,
                visited = true,
                tags = { "tower", "landmark" },
            }
            local s = json:stringify(input)
            local out = json:parse(s)
            assert(out.name == input.name, "name")
            assert(out.lon == input.lon, "lon")
            assert(out.lat == input.lat, "lat")
            assert(out.zoom == input.zoom, "zoom")
            assert(out.visited == input.visited, "visited")
            assert(#out.tags == 2, "tags len")
            assert(out.tags[1] == "tower", "tags[1]")
            assert(out.tags[2] == "landmark", "tags[2]")
            "#,
        )
        .exec()
        .unwrap();
    }
}
