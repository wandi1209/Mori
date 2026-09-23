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
        local S = { blocks = 0, seeds = 0, world = "", sleeps = 0, log = {} }
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
          tiles[i] = { x = 10 + i, y = 24, fg = (i <= 3) and 2019 or 0,
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
        function bot:isInTile() return true end
        function bot:findPath() end
        function bot:setAutoCollect(on) S.collect = on end
        function bot:stopScript() error("STOPSCRIPT") end
        function bot:warp(n) S.world = n; S.log[#S.log+1] = "warp:" .. n end
        function bot:drop(id, n) S.seeds = S.seeds - n; S.log[#S.log+1] = "drop:" .. n end
        function bot:hit(x, y)
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
        function bot:place(x, y, id)
          local t = tileAt(x, y)
          if not t or t.fg ~= 0 then return end
          t.fg = id
          if id == 2018 then S.blocks = S.blocks - 1
          elseif id == 2019 then S.seeds = S.seeds - 1; S.log[#S.log+1] = "plant" end
        end
        function getBot() return bot end

        function sleep(ms)
          S.sleeps = S.sleeps + 1
          if ms >= 60000 then error("CYCLE_DONE") end   -- the end-of-cycle wait
        end
    "#;

    #[test]
    fn farm_script_completes_a_cycle() {
        let lua = mlua::Lua::new_with(
            mlua::StdLib::TABLE | mlua::StdLib::STRING | mlua::StdLib::MATH | mlua::StdLib::IO,
            mlua::LuaOptions::default(),
        )
        .expect("lua init failed");
        lua.load(STUBS).exec().expect("stub world failed to load");

        let src = std::fs::read_to_string("scripts/farm.lua")
            .expect("scripts/farm.lua missing")
            .replace("\"YOURFARM\"", "\"SIMWORLD\"");

        let err = lua.load(&src).exec().unwrap_err().to_string();
        assert!(err.contains("CYCLE_DONE"), "script stopped early: {err}");

        let log: Vec<String> = lua.load("return SIM.log").eval().unwrap();
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

        // The drop lands at the bot's feet, so collecting must be off while it
        // happens and back on once the bot is home.
        let dropped_at = log.iter().position(|l| l.starts_with("drop:")).unwrap();
        let home_after_drop = log[dropped_at..].iter().any(|l| l == "warp:SIMWORLD");
        assert!(home_after_drop, "bot should return to the farm after dumping");
        let collect_on: bool = lua.load("return SIM.collect").eval().unwrap();
        assert!(collect_on, "auto-collect should be back on in the farm world");
    }
}
