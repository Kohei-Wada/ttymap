//! `ttymap.help` userdata — snapshot data the help plugin renders.

use std::sync::Arc;

use mlua::UserData;

use crate::lua::host::LuaHostShared;

pub(super) struct HostHelp {
    shared: Arc<LuaHostShared>,
}

impl HostHelp {
    pub(super) fn new(shared: Arc<LuaHostShared>) -> Self {
        Self { shared }
    }
}

impl UserData for HostHelp {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `ttymap.help:keymap_entries() -> [{key, label}, …]` —
        // keybindings for built-in map actions, formatted for
        // help-style display. Read at every call so post-init.lua
        // population is visible.
        methods.add_method("keymap_entries", |lua, this, _: ()| {
            let table = lua.create_table()?;
            let entries = this.shared.keymap_entries.lock();
            let entries = match &entries {
                Ok(g) => g.as_slice(),
                Err(_) => &[],
            };
            for (i, (key, label)) in entries.iter().enumerate() {
                let row = lua.create_table()?;
                row.set("key", key.as_str())?;
                row.set("label", label.as_str())?;
                table.set(i + 1, row)?;
            }
            Ok(table)
        });

        // `ttymap.help:palette_entries() -> [{name, key, label}, …]`
        // — snapshot of every plugin's metadata, appended during
        // registration. Read lazily so help can be loaded mid-
        // registration and still see every sibling at render time.
        // Returns an empty list when the snapshot hasn't been
        // populated yet.
        methods.add_method("palette_entries", |lua, this, _: ()| {
            let table = lua.create_table()?;
            let entries = this.shared.palette_entries.lock();
            let entries = match &entries {
                Ok(g) => g.as_slice(),
                Err(_) => &[],
            };
            for (i, entry) in entries.iter().enumerate() {
                let row = lua.create_table()?;
                row.set("name", entry.name.as_str())?;
                row.set("key", entry.key.as_str())?;
                row.set("label", entry.label.as_str())?;
                table.set(i + 1, row)?;
            }
            Ok(table)
        });
    }
}
