mod http;
mod runtime;
mod types;
mod webhook;

pub use runtime::run_script_threaded;

#[cfg(test)]
mod tests {
    /// The scripts shipped in `scripts/` have to parse in the same sandbox the
    /// bots run them in — a typo there is otherwise only found at run time, by
    /// whoever is farming.
    #[test]
    fn bundled_scripts_parse() {
        let lua = mlua::Lua::new_with(
            mlua::StdLib::TABLE | mlua::StdLib::STRING | mlua::StdLib::MATH | mlua::StdLib::IO,
            mlua::LuaOptions::default(),
        )
        .expect("lua init failed");

        let dir = std::path::Path::new("scripts");
        if !dir.is_dir() {
            return;
        }

        let mut checked = 0;
        for entry in std::fs::read_dir(dir).expect("cannot read scripts/") {
            let path = entry.expect("bad dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("lua") {
                continue;
            }
            let src = std::fs::read_to_string(&path).expect("cannot read script");
            lua.load(&src)
                .set_name(path.display().to_string())
                .into_function()
                .unwrap_or_else(|e| panic!("{} failed to parse: {e}", path.display()));
            checked += 1;
        }
        assert!(checked > 0, "no scripts found to check");
    }
}
