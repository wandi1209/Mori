-- Crop farm loop, for any block that grows on a tree.
--
--   harvest until the block stack fills
--     -> break those blocks back into seeds
--       -> plant every free plot
--         -> go round again while trees are still ripe
--           -> dump surplus seeds, then poll until something ripens
--
-- One crop per bot: run this script on the account that farms that block, with
-- `crop` set to its name. A second block means a second account running the same
-- file with a different name in it.
--
-- Ids, growth time and block strength are read from items.dat at startup via
-- getInfo(), so the crop is named, not numbered: "Dark Yellow Block" resolves to
-- block 2018, seed 2019, 43m07s; "Dark Purple Block" to 2026, 2027, 5h06m. Any
-- tree-grown block works, since its seed is always "<name> Seed".
--
-- EDIT THE CROP AND WORLD NAMES BELOW. Everything else has working defaults.
--
-- Two things worth knowing before the first run:
--   * print() goes to the terminal running Mori, not the bot's web console.
--   * Breaking a block does not reliably return a seed, and often returns the
--     block itself, so the break phase is capped by max_break_rounds rather
--     than running until the blocks are gone. Watch one cycle and tune it.

local CONFIG = {
  crop     = "Dark Yellow Block",  -- exact items.dat name; "Dark Purple Block" etc
  world    = "YOURFARM",           -- world the trees live in
  world_id = "",                   -- door id, "" for the main entrance
  area     = { x1 = 1, y1 = 1, x2 = 98, y2 = 3 },  -- plots to farm, inclusive

  -- Layered farms leave walkways between planting rows. With row_step = 2 only
  -- every second row inside the area is planted, counted from y1 + row_offset;
  -- row_step = 1 plants every empty tile in the box.
  row_step   = 1,
  row_offset = 0,

  -- Where surplus seeds go. Leave dump_world empty to keep them in the farm
  -- world, dropped on dump_spot — a tile away from the plots, since the bot has
  -- to stand clear of the pile for auto-collect not to pick it straight back up.
  dump_world    = "YOURSTORE",
  dump_world_id = "",
  dump_spot     = { x = 54, y = 21 },
  seed_dump_at  = 50,           -- surplus seeds that trigger a dump run
  seed_keep     = 10,           -- seeds kept back after a dump

  -- Where blocks are placed and broken back into seeds. One tile, reused every
  -- round, so the bot stands still instead of running across the farm for each
  -- of a few hundred blocks. Keep it out of `area`: a block that survives its
  -- hit budget stays on the tile, and inside the farm that plot is then never
  -- planted again. Set to nil to break on random empty plots instead.
  break_spot = { x = 2, y = 9 },

  -- Guard rails.
  stack_cap        = 200,   -- a stack of one item never goes past this in-game
  stall_limit      = 5,     -- harvested trees in a row that add no blocks before
                            -- the script decides the drops are not arriving
  max_passes       = 8,     -- harvest+break rounds per cycle; a farm bigger than
                            -- the 200-block stack cap needs more than one
  max_break_rounds = 400,   -- place-and-break attempts per cycle
  extra_hits       = 4,     -- punches on top of the item's own strength
  step_timeout_ms  = 8000,  -- giving up on a walk or a warp
  -- Extra pause after each punch or placement. The bot already waits place_ms
  -- after every one of them, with its own jitter, so this is on top of that —
  -- leave it at 0 and tune the delays in the bot's Config tab instead.
  action_delay_ms  = 0,
  cycle_jitter_s   = { 30, 300 }, -- random tail added to every wait
  idle_recheck_s   = 300,   -- how long to wait before looking again when nothing
                            -- is ready. The loop never sleeps out a growth timer:
                            -- trees planted at different moments ripen at
                            -- different moments, so it polls instead
}

-- ── helpers ────────────────────────────────────────────────────────────────

local bot = getBot()

--- Seconds of sleep this script has accumulated. The sandbox has no os.time, so
--- every timer in here is measured against this counter.
local elapsed = 0

local function log(msg)
  print("[farm] " .. msg)
end

--- sleep() that keeps the elapsed counter honest. Use this, never sleep().
local function nap(ms)
  sleep(ms)
  elapsed = elapsed + ms / 1000
end

local function inv(id)
  return getInventory():findItem(id)
end

--- The tile the bot is standing on.
local function botTile()
  local me = getLocal()
  return math.floor(me.posx / 32), math.floor(me.posy / 32)
end

--- bot:hit and bot:place take an offset from the tile the bot is standing on,
--- capped at four tiles in each direction — not absolute coordinates. Sending
--- absolute ones punches whatever happens to sit that far away, or nothing at
--- all. Everything else in this script works in absolute tiles, so the
--- conversion lives here.
local function offsetTo(x, y)
  local me = getLocal()
  return x - math.floor(me.posx / 32), y - math.floor(me.posy / 32)
end

local function hitTile(x, y)
  local dx, dy = offsetTo(x, y)
  bot:hit(dx, dy)
end

local function placeTile(x, y, item)
  local dx, dy = offsetTo(x, y)
  bot:place(dx, dy, item)
end

--- Looks the crop's numbers up in items.dat. Returns nil when the name is wrong,
--- which is the likeliest configuration mistake.
local function resolveCrop(name)
  local block = getInfo(name)
  local seed  = getInfo(name .. " Seed")

  if not block or not seed then
    log("unknown crop: " .. tostring(name))
    return nil
  end

  return {
    name     = name,
    block_id = block.id,
    seed_id  = seed.id,
    grow     = seed.grow_time,
    hits     = block.strength + CONFIG.extra_hits,
  }
end

--- Walks to a tile and waits until the bot is standing on it.
--- Returns false when the bot never arrives, so callers can skip that plot.
local function goTo(x, y)
  if bot:isInTile(x, y) then return true end
  bot:findPath(x, y)

  local waited = 0
  while waited < CONFIG.step_timeout_ms do
    if bot:isInTile(x, y) then return true end
    nap(100)
    waited = waited + 100
  end
  return false
end

--- Puts the bot beside `x, y` rather than on top of it, so punches and
--- placements go to the tile in front of the character the way a player's do.
--- Falls back to standing on the tile when nothing next to it can be reached.
local function goNextTo(x, y)
  local me = getLocal()
  local bx, by = math.floor(me.posx / 32), math.floor(me.posy / 32)
  if math.abs(bx - x) + math.abs(by - y) == 1 then
    return true
  end

  for _, d in ipairs({ { -1, 0 }, { 1, 0 }, { 0, -1 }, { 0, 1 } }) do
    local nx, ny = x + d[1], y + d[2]
    local tile = getTile(nx, ny)
    if tile and tile.fg == 0 and goTo(nx, ny) then
      return true
    end
  end

  return goTo(x, y)
end

--- Warps and waits for the world to load. Returns false if it never arrives.
local function goToWorld(name, id)
  if bot:isInWorld(name) then return true end
  log("warping to " .. name)
  bot:warp(name, id)

  local waited = 0
  while waited < CONFIG.step_timeout_ms do
    if bot:isInWorld(name) then
      nap(1000) -- let the map finish loading before reading tiles
      return true
    end
    nap(250)
    waited = waited + 250
  end
  log("warp to " .. name .. " failed")
  return false
end

--- Tiles inside a crop's area that satisfy `pred(tile)`.
local function plots(pred)
  local found = {}
  local a = CONFIG.area
  for _, tile in ipairs(getTiles()) do
    if tile.x >= a.x1 and tile.x <= a.x2 and tile.y >= a.y1 and tile.y <= a.y2 then
      if pred(tile) then
        found[#found + 1] = { x = tile.x, y = tile.y }
      end
    end
  end
  return found
end

--- Any ready-to-harvest tree in the area. Deliberately not filtered by item id:
--- in a dedicated farm world everything ready is worth picking.
local function readyTrees()
  return plots(function(t) return t:canHarvest() end)
end

--- Rows the farm plants on. Everything else inside the area is left alone, which
--- is what keeps walkways walkable on a layered farm.
local function isPlantingRow(y)
  local step = CONFIG.row_step or 1
  if step <= 1 then return true end
  return (y - CONFIG.area.y1 - (CONFIG.row_offset or 0)) % step == 0
end

local function emptyPlots()
  return plots(function(t) return t.fg == 0 and isPlantingRow(t.y) end)
end

--- Punches a tile until it clears, or until the hit budget runs out. Used for
--- both harvesting a tree and breaking a placed block.
local function punchUntilClear(crop, x, y)
  for _ = 1, crop.hits do
    hitTile(x, y)
    nap(CONFIG.action_delay_ms)

    local tile = getTile(x, y)
    if tile and tile.fg == 0 then return true end
  end
  return false
end

-- ── phases ─────────────────────────────────────────────────────────────────

--- Harvests ready trees until none are left or the block stack fills up.
--- Returns how many trees were actually punched, for the cycle summary.
--- Returns how many trees were picked and the tile of the first one, so the rest
--- of the pass can start where the bot already is instead of walking back to the
--- corner of the farm.
local function harvest(crop)
  local trees = readyTrees()
  local picked = 0
  local first_x, first_y
  log(crop.name .. " harvest: " .. #trees .. " ready, "
    .. inv(crop.block_id) .. " blocks held")

  local stalled = 0

  for _, plot in ipairs(trees) do
    local held = inv(crop.block_id)
    local bag = getInventory()

    if held >= CONFIG.stack_cap or not bag:canCollect(crop.block_id) then
      log(crop.name .. " harvest: stack full at " .. held
        .. ", switching to break phase")
      return picked, first_x, first_y
    end

    -- A full backpack does not stop a drop from falling, it stops it from being
    -- picked up: the count never moves and harvesting would run the farm down
    -- for nothing. Watch for progress rather than trusting one flag.
    if bag.itemcount >= bag.slotcount and held == 0 then
      log(crop.name .. " harvest: backpack is out of slots, nothing can be picked up")
      return picked, first_x, first_y
    end

    if not bot:isInWorld(CONFIG.world) then return picked, first_x, first_y end

    if goNextTo(plot.x, plot.y) then
      punchUntilClear(crop, plot.x, plot.y)
      picked = picked + 1
      first_x = first_x or plot.x
      first_y = first_y or plot.y

      if inv(crop.block_id) <= held then
        stalled = stalled + 1
        if stalled >= CONFIG.stall_limit then
          log(crop.name .. " harvest: " .. stalled
            .. " trees in a row added no blocks, stopping - the drops are not"
            .. " reaching the bag (full backpack, or auto-collect is off)")
          return picked
        end
      else
        stalled = 0
      end
    end
  end
  return picked, first_x, first_y
end

--- Places blocks back down and breaks them, which is what yields seeds.
--- Bounded by max_break_rounds: a block does not always drop a seed, and a
--- broken block often comes straight back, so this can never be "until done".
local function breakBlocks(crop)
  local rounds = 0

  while inv(crop.block_id) > 0 and rounds < CONFIG.max_break_rounds do
    if not bot:isInWorld(CONFIG.world) then return end

    -- The dedicated spot, as long as it is clear; otherwise fall back to any
    -- empty plot, which also covers a block left standing on the spot itself.
    local plot = CONFIG.break_spot
    local spot_tile = plot and getTile(plot.x, plot.y)
    if not plot or (spot_tile and spot_tile.fg ~= 0) then
      local free = emptyPlots()
      if #free == 0 then
        log(crop.name .. " break: no free tile to place on")
        return
      end
      plot = free[math.random(#free)]
    end

    if goNextTo(plot.x, plot.y) then
      placeTile(plot.x, plot.y, crop.block_id)
      nap(CONFIG.action_delay_ms)
      punchUntilClear(crop, plot.x, plot.y)
      -- Auto-collect picks the drops up; give it a tick to do so.
      nap(CONFIG.action_delay_ms)
    end
    rounds = rounds + 1
  end

  log(crop.name .. " break: " .. rounds .. " rounds, "
    .. inv(crop.seed_id) .. " seeds held")
end

--- Plants one seed on every free plot, until the seeds run out.
--- Plants every free plot, starting at `from_x, from_y` and wrapping around.
--- Plots come back in world order, which starts at the top-left corner however
--- far away that is; beginning where the harvest did saves the walk back.
local function plant(crop, from_x, from_y)
  local free = emptyPlots()

  if from_x and from_y and #free > 1 then
    local start = 1
    for i, plot in ipairs(free) do
      if plot.y > from_y or (plot.y == from_y and plot.x >= from_x) then
        start = i
        break
      end
    end
    local rotated = {}
    for i = 0, #free - 1 do
      rotated[#rotated + 1] = free[((start - 1 + i) % #free) + 1]
    end
    free = rotated
  end

  local planted, unreachable, refused = 0, 0, 0
  log(crop.name .. " plant: " .. #free .. " free plots, "
    .. inv(crop.seed_id) .. " seeds held")

  for _, plot in ipairs(free) do
    if inv(crop.seed_id) <= 0 then break end
    if not bot:isInWorld(CONFIG.world) then break end

    if not goNextTo(plot.x, plot.y) then
      unreachable = unreachable + 1
    else
      placeTile(plot.x, plot.y, crop.seed_id)
      nap(CONFIG.action_delay_ms)

      -- The server is free to ignore a placement — out of build range, no
      -- access in that world, or a tile that is not what the snapshot said.
      -- Counting what actually took keeps a silent refusal from looking like
      -- a planted farm.
      local tile = getTile(plot.x, plot.y)
      if tile and tile.fg ~= 0 then
        planted = planted + 1
      else
        refused = refused + 1
      end
    end
  end

  if unreachable > 0 or refused > 0 then
    log(crop.name .. " plant: " .. planted .. " planted, "
      .. unreachable .. " unreachable, " .. refused .. " refused by the server")
  end
  return planted
end

--- Drops surplus seeds, keeping `seed_keep` back.
---
--- Auto-collect goes off before the bot goes anywhere near the pile: a drop lands
--- at its feet, and arriving at an old pile with collecting on would scoop the
--- whole store back up. It comes back on once the bot has left the spot.
local function dumpSeeds(crop)
  local held = inv(crop.seed_id)
  local surplus = held - CONFIG.seed_keep
  if held < CONFIG.seed_dump_at or surplus <= 0 then return end

  local same_world = CONFIG.dump_world == nil or CONFIG.dump_world == ""
  local where = same_world and "the storage spot" or CONFIG.dump_world
  log(crop.name .. " dump: " .. surplus .. " seeds to " .. where)

  bot:setAutoCollect(false)

  if same_world then
    local spot = CONFIG.dump_spot
    if not spot or not goTo(spot.x, spot.y) then
      log("dump: could not reach the storage spot")
      bot:setAutoCollect(true)
      return
    end
    bot:drop(crop.seed_id, surplus)
    nap(2000)
    -- Step off the pile before collecting resumes.
    goTo(CONFIG.area.x1, CONFIG.area.y1)
    bot:setAutoCollect(true)
    return
  end

  if not goToWorld(CONFIG.dump_world, CONFIG.dump_world_id) then
    bot:setAutoCollect(true)
    return
  end
  -- Never drop into whatever world the bot happens to be standing in.
  if not bot:isInWorld(CONFIG.dump_world) then
    log("dump: not in " .. CONFIG.dump_world .. ", skipping")
    bot:setAutoCollect(true)
    return
  end

  bot:drop(crop.seed_id, surplus)
  nap(2000)

  -- Head home so the growth wait is spent in the farm, not on top of the pile.
  if goToWorld(CONFIG.world, CONFIG.world_id) then
    bot:setAutoCollect(true)
  end
end

local function runCycle(crop)
  if not goToWorld(CONFIG.world, CONFIG.world_id) then return false end
  bot:setAutoCollect(true)

  -- Everything needed to answer "does this farm pay for itself": seeds returned
  -- per tree has to reach 1, or the seed stock shrinks every cycle no matter how
  -- many plots there are.
  local seeds_before = inv(crop.seed_id)
  local harvested, broken_from, planted = 0, 0, 0
  local stuck = false

  -- Where the bot stood when the cycle began. It goes back there at the end, so
  -- a farm left idle has its bot parked in one place rather than wherever the
  -- last plot happened to be.
  local home_x, home_y = botTile()

  -- One pass is harvest, break, plant. Harvesting stops at the 200-block stack
  -- cap, so a farm that yields more than that takes several passes: empty the
  -- stack into seeds, put those seeds in the ground, then go back for the trees
  -- that are still standing.
  for pass = 1, CONFIG.max_passes do
    local picked, first_x, first_y = harvest(crop)
    harvested = harvested + picked

    broken_from = broken_from + inv(crop.block_id)
    breakBlocks(crop)
    planted = planted + plant(crop, first_x, first_y)

    -- Blocks still at the cap means breaking did not drain them, and harvesting
    -- cannot resume until it does. Going round again would only walk the farm.
    if not getInventory():canCollect(crop.block_id) then
      log(crop.name .. " break: stack still full (" .. inv(crop.block_id)
        .. " blocks) - check break_spot, it needs a reachable empty tile")
      stuck = true
      break
    end

    if picked == 0 or #readyTrees() == 0 then break end
    log(crop.name .. " pass " .. pass .. " done, more trees are still ready")
  end

  dumpSeeds(crop)
  goTo(home_x, home_y)

  if stuck then return false end

  if harvested == 0 then
    log(string.format("%s cycle: nothing ready, %d replanted", crop.name, planted))
    return false
  end

  local seeds_gained = inv(crop.seed_id) - seeds_before + planted
  local per_tree = seeds_gained / harvested
  log(string.format(
    "%s cycle: %d trees -> %d blocks -> %d seeds (%.2f seeds/tree), %d replanted%s",
    crop.name, harvested, broken_from, seeds_gained, per_tree, planted,
    per_tree >= 1 and " [sustains itself]" or " [losing seeds]"))
  return true
end

-- ── main loop ──────────────────────────────────────────────────────────────

local crop = resolveCrop(CONFIG.crop)
if not crop then
  log("nothing to farm, stopping")
  bot:stopScript()
  return
end

log(crop.name .. ": block " .. crop.block_id .. ", seed " .. crop.seed_id
  .. ", grows in " .. crop.grow .. "s, " .. crop.hits .. " hits to break")

math.randomseed(crop.block_id + inv(crop.seed_id) + 1)

while true do
  local harvested_something = runCycle(crop)

  -- Go straight into the next cycle when the farm has more ready trees: a stack
  -- cap can end a cycle early, and trees planted at different moments ripen at
  -- different moments, so a fixed growth-timer sleep would leave them standing.
  -- Requiring that this cycle harvested something keeps an unreachable ready
  -- tree from spinning the loop.
  local more_ready = harvested_something
    and bot:isInWorld(CONFIG.world)
    and #readyTrees() > 0

  if more_ready then
    log("more trees are ready, going again")
  else
    local jitter = math.random(CONFIG.cycle_jitter_s[1], CONFIG.cycle_jitter_s[2])
    local wait = CONFIG.idle_recheck_s + jitter
    log("nothing ripe, checking again in " .. math.floor(wait) .. "s")
    nap(wait * 1000)
  end
end
