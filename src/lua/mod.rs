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

#[cfg(test)]
mod farm_script_tests {
    //! The farm script cannot be exercised against a live server from a test, so
    //! it runs here against a stub world: six plots, three of them ready trees,
    //! an inventory that counts blocks and seeds, and a sleep that aborts the
    //! script once it reaches the end-of-cycle wait.

    const STUBS: &str = r#"
        local S = { blocks = 0, seeds = 0, world = "", sleeps = 0, log = {}, printed = {},
                    x = 0, y = 0 }
        _G.SIM = S

        local ITEMS = {
          ["Dark Yellow Block"]      = { id = 2018, grow_time = 0,     strength = 4 },
          ["Dark Yellow Block Seed"] = { id = 2019, grow_time = 2587,  strength = 0 },
          ["Dark Purple Block"]      = { id = 2026, grow_time = 0,     strength = 4 },
          ["Dark Purple Block Seed"] = { id = 2027, grow_time = 18356, strength = 0 },
        }
        function getInfo(name) return ITEMS[name] end

        -- 6 plots: 3 ready trees, 3 empty
        local tiles = {}
        for i = 1, 6 do
          -- rows 24 and 25, so a row_step of 2 would halve the plantable set
          tiles[i] = { x = 10 + i, y = 24 + (i % 2), fg = (i <= 3) and 2019 or 0,
                       ready = i <= 3 }
          tiles[i].canHarvest = function(self) return self.ready end
        end
        local function tileAt(x, y)
          for _, t in ipairs(tiles) do if t.x == x and t.y == y then return t end end
        end
        function getTiles() return tiles end
        function getTile(x, y) return tileAt(x, y) end

        function getInventory()
          return {
            findItem = function(_, id)
              if id == 2018 then return S.blocks elseif id == 2019 then return S.seeds end
              return 0
            end,
            canCollect = function(_, id)
              if id == 2018 then return S.blocks < 200 end
              return true
            end,
          }
        end

        local bot = {}
        function bot:isInWorld(n) return S.world == n end
        function bot:isInTile(x, y) return S.x == x and S.y == y end
        function bot:findPath(x, y) S.x, S.y = x, y end
        function getLocal() return { posx = S.x * 32, posy = S.y * 32 } end
        function bot:setAutoCollect(on) S.collect = on end
        function bot:stopScript() error("STOPSCRIPT") end
        function bot:warp(n) S.world = n; S.log[#S.log+1] = "warp:" .. n end
        function bot:drop(id, n) S.seeds = S.seeds - n; S.log[#S.log+1] = "drop:" .. n end
        function bot:hit(dx, dy)
          local x, y = S.x + dx, S.y + dy
          local t = tileAt(x, y)
          if not t then return end
          if t.fg == 2019 and t.ready then            -- harvest a tree
            t.fg, t.ready = 0, false
            S.blocks = S.blocks + 60
            S.log[#S.log+1] = "harvest"
          elseif t.fg == 2018 then                    -- break a placed block
            t.fg = 0
            S.seeds = S.seeds + 1
            S.log[#S.log+1] = "break"
          end
        end
        function bot:place(dx, dy, id)
          local x, y = S.x + dx, S.y + dy
          local t = tileAt(x, y)
          if not t or t.fg ~= 0 then return end
          t.fg = id
          if id == 2018 then S.blocks = S.blocks - 1; S.log[#S.log+1] = "place:" .. x .. "," .. y
          elseif id == 2019 then S.seeds = S.seeds - 1; S.log[#S.log+1] = "plant" end
        end
        function getBot() return bot end

        local real_print = print
        function print(msg) S.printed[#S.printed+1] = tostring(msg); real_print(msg) end

        function sleep(ms)
          S.sleeps = S.sleeps + 1
          if ms >= 60000 then error("CYCLE_DONE") end   -- the end-of-cycle wait
        end
    "#;

    /// Every `lua.load(r#"..."#)` block in the runtime is Lua source that has to
    /// compile in the interpreter Mori actually embeds. One that does not takes
    /// the whole prelude with it, and with it every script on every bot — which is
    /// what a generic-for control variable assignment did under Lua 5.5, where
    /// those variables are const.
    #[test]
    fn the_lua_prelude_compiles() {
        let lua = mlua::Lua::new_with(
            mlua::StdLib::TABLE | mlua::StdLib::STRING | mlua::StdLib::MATH | mlua::StdLib::IO,
            mlua::LuaOptions::default(),
        )
        .expect("lua init failed");

        let src = std::fs::read_to_string("src/lua/runtime.rs").expect("runtime.rs missing");
        let mut checked = 0;
        let mut rest = src.as_str();
        while let Some(start) = rest.find("lua.load(r#\"") {
            let body = &rest[start + "lua.load(r#\"".len()..];
            let end = body.find("\"#").expect("unterminated lua.load block");
            let chunk = &body[..end];

            lua.load(chunk)
                .set_name(format!("prelude block {}", checked + 1))
                .into_function()
                .unwrap_or_else(|e| panic!("prelude block {} does not compile: {e}", checked + 1));

            checked += 1;
            rest = &body[end..];
        }
        assert!(checked >= 2, "expected to find the prelude blocks, found {checked}");
    }

    /// Overwrites one `key = value,` line of the script's CONFIG block. The block is
    /// meant to be edited for each farm, so tests set what they need rather than
    /// depending on the values the file happens to ship with.
    fn set_config(src: &str, key: &str, value: &str) -> String {
        let pattern = format!(r"(?m)^(\s*{key}\s*=\s*)(\{{[^}}]*\}}|[^,\n]*),");
        let re = regex::Regex::new(&pattern).expect("bad config pattern");
        assert!(re.is_match(src), "config key not found: {key}");
        re.replace(src, format!("${{1}}{value},")).into_owned()
    }

    /// Loads the farm script into a fresh stub world, optionally patched, and runs
    /// it until the end-of-cycle sleep aborts it. Returns the stub's action log.
    fn run_script(patch: &[(&str, &str)]) -> (mlua::Lua, Vec<String>) {
        let lua = mlua::Lua::new_with(
            mlua::StdLib::TABLE | mlua::StdLib::STRING | mlua::StdLib::MATH | mlua::StdLib::IO,
            mlua::LuaOptions::default(),
        )
        .expect("lua init failed");
        lua.load(STUBS).exec().expect("stub world failed to load");

        let mut src = std::fs::read_to_string("scripts/farm.lua")
            .expect("scripts/farm.lua missing");

        // Point the script at the stub world, whatever the file is configured for.
        src = set_config(&src, "world", "\"SIMWORLD\"");
        src = set_config(&src, "world_id", "\"\"");
        src = set_config(&src, "dump_world", "\"YOURSTORE\"");
        src = set_config(&src, "area", "{ x1 = 0, y1 = 0, x2 = 99, y2 = 59 }");
        src = set_config(&src, "row_step", "1");
        src = set_config(&src, "break_spot", "nil");

        for (key, value) in patch {
            src = set_config(&src, key, value);
        }

        let err = lua.load(&src).exec().unwrap_err().to_string();
        assert!(err.contains("CYCLE_DONE"), "script stopped early: {err}");

        let log: Vec<String> = lua.load("return SIM.log").eval().unwrap();
        (lua, log)
    }

    #[test]
    fn an_idle_cycle_waits_the_short_interval() {
        // No ready trees: the stub's plots are all empty to start with.
        // area excludes the stub's tiles, so nothing is ready and nothing is empty
        let (lua, _) = run_script(&[
            ("idle_recheck_s", "120"),
            ("area", "{ x1 = 80, y1 = 50, x2 = 85, y2 = 55 }"),
        ]);
        let printed: Vec<String> = lua.load("return SIM.printed").eval().unwrap();

        assert!(
            printed.iter().any(|l| l.contains("nothing ready")),
            "an empty farm should say so: {printed:?}"
        );
        let slept: u64 = printed
            .iter()
            .find_map(|l| {
                l.strip_prefix("[farm] nothing ripe, checking again in ")?
                    .strip_suffix("s")?
                    .parse()
                    .ok()
            })
            .expect("no wait line");
        assert!(
            (120..=420).contains(&slept),
            "should wait the idle interval plus jitter, waited {slept}s"
        );
    }

    #[test]
    fn break_spot_keeps_the_bot_on_one_tile() {
        // x 14, y 24 is one of the stub's empty plots.
        let (_, log) = run_script(&[("break_spot", "{ x = 14, y = 24 }")]);

        let places: Vec<&String> = log.iter().filter(|l| l.starts_with("place:")).collect();
        assert!(places.len() > 1, "blocks should have been placed");
        assert!(
            places.iter().all(|l| l.as_str() == "place:14,24"),
            "every block should be placed on the break spot, got {places:?}"
        );
    }

    #[test]
    fn seeds_can_be_stored_in_the_farm_world() {
        let (lua, log) = run_script(&[("dump_world", "\"\"")]);

        assert!(
            log.iter().any(|l| l.starts_with("drop:")),
            "surplus seeds should still be dropped"
        );
        assert!(
            !log.iter().any(|l| l == "warp:YOURSTORE"),
            "no warp should happen when storing in the farm world"
        );
        let collect_on: bool = lua.load("return SIM.collect").eval().unwrap();
        assert!(collect_on, "auto-collect should be back on after the drop");
    }

    #[test]
    fn row_step_skips_the_walkways() {
        // Six plots spread over rows 24 and 25; three hold trees, three are empty.
        let (_, every_row) = run_script(&[]);
        let (_, alternate) = run_script(&[("row_step", "2")]);

        let planted = |log: &[String]| log.iter().filter(|l| l.as_str() == "plant").count();
        assert!(planted(&every_row) > planted(&alternate));
        assert!(planted(&alternate) > 0, "the planting row should still be used");
    }

    #[test]
    fn farm_script_completes_a_cycle() {
        let (lua, log) = run_script(&[]);
        let printed: Vec<String> = lua.load("return SIM.printed").eval().unwrap();
        let blocks: i64 = lua.load("return SIM.blocks").eval().unwrap();
        let seeds: i64 = lua.load("return SIM.seeds").eval().unwrap();
        let count = |what: &str| log.iter().filter(|l| l.as_str() == what).count();

        assert_eq!(count("harvest"), 3, "every ready tree should be harvested");
        assert!(count("break") > 0, "blocks should be broken back into seeds");
        assert_eq!(count("plant"), 6, "every free plot should be replanted");
        assert_eq!(blocks, 0, "blocks should end the cycle converted");
        assert_eq!(seeds, 10, "seed_keep seeds should survive the dump");
        assert!(
            log.iter().any(|l| l.starts_with("drop:")),
            "surplus seeds should be dropped in the storage world"
        );
        assert!(
            printed.iter().any(|l| l.contains("seeds/tree")),
            "the cycle summary should report the seed return per tree"
        );

        // The drop lands at the bot's feet, so collecting must be off while it
        // happens and back on once the bot is home.
        let dropped_at = log.iter().position(|l| l.starts_with("drop:")).unwrap();
        let home_after_drop = log[dropped_at..].iter().any(|l| l == "warp:SIMWORLD");
        assert!(home_after_drop, "bot should return to the farm after dumping");
        let collect_on: bool = lua.load("return SIM.collect").eval().unwrap();
        assert!(collect_on, "auto-collect should be back on in the farm world");
    }
}
