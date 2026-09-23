# Scripts

Lua scripts for the bot script tab. Paste one in, edit the `CONFIG` block at the
top, run it.

| Script | What it does |
|--------|--------------|
| `dark-yellow-farm.lua` | Full Dark Yellow Block cycle: harvest ready trees, break blocks back into seeds once the stack caps out, replant every free plot, dump surplus seeds in a storage world, then wait out the 43m07s growth timer and repeat. |

Notes that apply to all of them:

* The Lua sandbox loads `table`, `string`, `math` and `io` only. There is no `os`,
  so no `os.time`, `os.date` or `os.clock` — scripts measure time with `sleep`
  and their own counters. For wall-clock scheduling use the Active Hours feature
  instead, which lives in the bot's Config tab.
* `print()` writes to the terminal running Mori, not to the bot's web console.
* `scripts/*.lua` is syntax-checked by `cargo test bundled_scripts_parse`.
