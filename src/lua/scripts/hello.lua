-- ttymap built-in Lua plugin demo.
--
-- A plugin module is just a table with at least `name` and `render`
-- fields. `render()` is called every frame; return a list of strings
-- and the host wraps them in a framed Paragraph titled with `name`.
--
-- Future plugin authors: copy this file as a starting template.
-- Bridge surface today is intentionally tiny (text lines only); see
-- docs/lua-bridge-surface.md for what's coming.

return {
    name = "hello",
    render = function()
        return {
            "Hello from Lua!",
            "",
            "This panel is rendered by",
            "src/lua/scripts/hello.lua",
            "",
            "Enable in config:",
            "  [lua]",
            "  enabled = true",
        }
    end,
}
