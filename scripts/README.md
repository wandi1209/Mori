# Scripts

Lua scripts for the bot script tab. Paste one in, edit the `CONFIG` block at the
top, run it.

The files here ship with placeholder worlds and coordinates. Keep your own filled
-in version as `farm.local.lua`, which git ignores, so pulling an update to the
script never overwrites your farm's numbers.

| Script | What it does |
|--------|--------------|
| `area-probe.lua` | Read-only. Prints how many empty plots, ready trees and solid tiles fall inside a candidate `area`, and which rows hold plots at all — for checking the numbers that go into `farm.lua` instead of guessing them. |
| `farm.lua` | Full crop cycle for one tree-grown block. Each pass harvests until the block stack fills, breaks those blocks back into seeds, and plants every free plot; passes repeat while trees are still ripe. Surplus seeds are dumped at the end, then it polls until something ripens. |

## Configuring `farm.lua`

One crop per bot. Run the same file on each farming account with its own crop
name and world:

```lua
crop     = "Dark Yellow Block",   -- or "Dark Purple Block", or any other
world    = "YOURFARM",
area     = { x1 = 10, y1 = 24, x2 = 89, y2 = 48 },
```

### Which tiles get planted

`area` is an inclusive rectangle of tile coordinates, and by default every empty
tile inside it is planted — the script has no notion of rows, it plants whatever
is air:

```lua
area     = { x1 = 10, y1 = 24, x2 = 89, y2 = 48 },   -- columns 10-89, rows 24-48
row_step   = 1,   -- 2 = plant every second row, leaving walkways alone
row_offset = 0,   -- shifts which rows those are, counted from y1
```

On a layered farm — solid rows with air rows between them — the solid rows are
skipped anyway, since they are not empty. `row_step` is for when the air itself
alternates between planting rows and walkways: with `row_step = 2` only rows
`y1`, `y1+2`, `y1+4` … are used. Keep `area` tight around the farm; anything empty
inside it, including the space around a door or sign, counts as a plot.

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

### Where blocks get broken

Turning blocks back into seeds means placing one and punching it, a few hundred
times per cycle. `break_spot` is the tile that happens on:

```lua
break_spot = { x = 54, y = 22 },   -- nil to use random empty plots instead
```

Keep it outside `area`. A block that survives its hit budget stays where it was
placed, and on a farm plot that tile is then never planted again; on the break
spot the script simply falls back to another tile for that round. One fixed tile
also means the bot stands still for the break phase rather than walking to a new
plot for every block.

### Storing the surplus

Surplus seeds are dropped on the floor once the bot holds `seed_dump_at` of them,
keeping `seed_keep` back to replant with. They can go to a separate world:

```lua
dump_world    = "YOURSTORE",
dump_world_id = "",          -- door id; empty for the main entrance
seed_dump_at  = 50,
seed_keep     = 10,
```

or stay in the farm world, on a tile of your choosing:

```lua
dump_world = "",
dump_spot  = { x = 5, y = 24 },
```

Keep `dump_spot` out of `area`, or the pile sits among the plots and the bot
walks over it while farming. A drop is a pile on the ground that anyone standing
there can pick up, so the world holding it wants to be one the account owns and
has locked — an unlocked world is a giveaway, not storage.

The script turns auto-collect off before dropping — the pile lands at the bot's
feet and would otherwise be picked straight back up — warps home, and turns it
on again there. It also refuses to drop unless it confirms it is actually in the
storage world.

### When nothing is ready

The loop never sleeps out a growth timer. After a cycle it looks at the farm
again: anything ripe and it goes straight into the next one, otherwise it waits
`idle_recheck_s` (five minutes by default) and looks again.

```
Dark Yellow Block cycle: nothing ready, 0 replanted
nothing ripe, checking again in 431s
```

A fixed growth-timer sleep would be wrong twice over. Trees planted at different
moments ripen at different moments, so a single timer leaves some standing. And a
cycle that ends early on the 200-block stack cap has ripe trees waiting right
then — those are picked up immediately instead of after another timer.

Going again requires that the cycle harvested something, so a ready tree the bot
cannot reach makes it wait rather than spin.

### Walking

Plots come back in world order, which begins at the top-left corner of the area
however far that is from the bot. Each pass therefore remembers the first tree it
harvested and starts planting there, wrapping around the rest, so the bot carries
on from where it already is instead of walking back across the farm.

At the end of a cycle it returns to the tile it started on, which keeps an idle
bot parked in one place rather than standing wherever the last plot happened to
be.

### Pace

The bot paces itself: every punch and placement is followed by `place_ms` and
every tile walked by `walk_ms`, both randomised by the jitter percentage. Those
live in the bot's Config tab, not in the script, and they are what to change when
a farm is too slow or too obviously mechanical.

`action_delay_ms` in the script is an extra pause on top of those, and defaults
to 0 for that reason. At the stock 500ms delays, a tree that takes eight punches
costs about four seconds and crossing a 100-wide world about fifty.

### Sizing the farm

Every cycle ends with a line that answers whether the farm pays for itself:

```
Dark Yellow Block cycle: 96 trees -> 198 blocks -> 71 seeds (0.74 seeds/tree), 71 replanted [losing seeds]
```

A farm sustains itself when each tree returns at least one seed on average —
`seeds/tree >= 1`. That ratio does not depend on how many plots there are: plots
scale the throughput and the surplus, not the break-even. Below 1, no farm size
saves it; the seed stock shrinks every cycle until it runs out.

Plot count still matters for two other reasons. Small farms swing: a bad cycle can
empty the stock, which is what `seed_keep` cushions. And harvesting stops at the
200-block stack cap, so a farm yielding more than that per cycle is drained over
several passes — `max_passes` sets how many harvest-and-break rounds one cycle may
take.

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
