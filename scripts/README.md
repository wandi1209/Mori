# Scripts

Lua scripts for the bot script tab. Paste one in, edit the `CONFIG` block at the
top, run it.

| Script | What it does |
|--------|--------------|
| `farm.lua` | Full crop cycle for one tree-grown block: harvest ready trees, break blocks back into seeds once the stack fills, replant every free plot, dump surplus seeds in a storage world, then wait out the growth timer and repeat. |

## Configuring `farm.lua`

One crop per bot. Run the same file on each farming account with its own crop
name and world:

```lua
crop     = "Dark Yellow Block",   -- or "Dark Purple Block", or any other
world    = "YOURFARM",
area     = { x1 = 10, y1 = 24, x2 = 89, y2 = 48 },
```

Nothing else changes between crops. Ids, growth time and how many hits a block
takes are read from `items.dat` at startup, so the name is the only crop-specific
value:

| Crop | Block | Seed | Grows in |
|------|-------|------|----------|
| Dark Yellow Block | 2018 | 2019 | 43m07s |
| Dark Purple Block | 2026 | 2027 | 5h05m56s |

The name must match `items.dat` exactly and its seed must be `<name> Seed`, which
holds for every tree-grown block. A wrong name stops the script at startup with a
message rather than farming nothing silently.

`cargo test farm_script_completes_a_cycle` runs the script against a stub world —
six plots, three ready trees — and checks that a full harvest, break, plant and
dump cycle happens.

Notes that apply to all of them:

* The Lua sandbox loads `table`, `string`, `math` and `io` only. There is no `os`,
  so no `os.time`, `os.date` or `os.clock` — scripts measure time with `sleep`
  and their own counters. For wall-clock scheduling use the Active Hours feature
  instead, which lives in the bot's Config tab.
* `print()` writes to the terminal running Mori, not to the bot's web console.
* `scripts/*.lua` is syntax-checked by `cargo test bundled_scripts_parse`.
