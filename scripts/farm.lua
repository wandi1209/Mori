-- Crop farm loop, for any block that grows on a tree.
--
--   harvest ready trees
--     -> when the block stack fills, place and break blocks back into seeds
--       -> plant every free plot
--         -> dump surplus seeds in another world
--           -> wait out the growth timer and start over
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
  area     = { x1 = 10, y1 = 24, x2 = 89, y2 = 48 },  -- plots to farm, inclusive

  dump_world    = "YOURSTORE",  -- where surplus seeds are dropped
  dump_world_id = "",
  seed_dump_at  = 50,           -- surplus seeds that trigger a dump run
  seed_keep     = 10,           -- seeds kept back after a dump

  -- Guard rails.
  max_break_rounds = 400,   -- place-and-break attempts per cycle
  extra_hits       = 4,     -- punches on top of the item's own strength
  step_timeout_ms  = 8000,  -- giving up on a walk or a warp
  action_delay_ms  = 250,   -- pause after each punch or placement
  cycle_jitter_s   = { 30, 300 }, -- random tail added to every wait
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

local function emptyPlots()
  return plots(function(t) return t.fg == 0 end)
end

--- Punches a tile until it clears, or until the hit budget runs out. Used for
--- both harvesting a tree and breaking a placed block.
local function punchUntilClear(crop, x, y)
  for _ = 1, crop.hits do
    bot:hit(x, y)
    nap(CONFIG.action_delay_ms)

    local tile = getTile(x, y)
    if tile and tile.fg == 0 then return true end
  end
  return false
end

-- ── phases ─────────────────────────────────────────────────────────────────

--- Harvests ready trees until none are left or the block stack fills up.
local function harvest(crop)
  local trees = readyTrees()
  log(crop.name .. " harvest: " .. #trees .. " ready, "
    .. inv(crop.block_id) .. " blocks held")

  for _, plot in ipairs(trees) do
    if not getInventory():canCollect(crop.block_id) then
      log(crop.name .. " harvest: stack full, switching to break phase")
      return
    end
    if not bot:isInWorld(CONFIG.world) then return end

    if goTo(plot.x, plot.y) then
      punchUntilClear(crop, plot.x, plot.y)
    end
  end
end

--- Places blocks back down and breaks them, which is what yields seeds.
--- Bounded by max_break_rounds: a block does not always drop a seed, and a
--- broken block often comes straight back, so this can never be "until done".
local function breakBlocks(crop)
  local rounds = 0

  while inv(crop.block_id) > 0 and rounds < CONFIG.max_break_rounds do
    if not bot:isInWorld(CONFIG.world) then return end

    local free = emptyPlots()
    if #free == 0 then
      log(crop.name .. " break: no free plot to place on")
      return
    end

    local plot = free[math.random(#free)]
    if goTo(plot.x, plot.y) then
      bot:place(plot.x, plot.y, crop.block_id)
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
local function plant(crop)
  local free = emptyPlots()
  log(crop.name .. " plant: " .. #free .. " free plots, "
    .. inv(crop.seed_id) .. " seeds held")

  for _, plot in ipairs(free) do
    if inv(crop.seed_id) <= 0 then return end
    if not bot:isInWorld(CONFIG.world) then return end

    if goTo(plot.x, plot.y) then
      bot:place(plot.x, plot.y, crop.seed_id)
      nap(CONFIG.action_delay_ms)
    end
  end
end

--- Drops surplus seeds in the storage world, keeping `seed_keep` back.
local function dumpSeeds(crop)
  local held = inv(crop.seed_id)
  local surplus = held - CONFIG.seed_keep
  if held < CONFIG.seed_dump_at or surplus <= 0 then return end

  log(crop.name .. " dump: " .. surplus .. " seeds to " .. CONFIG.dump_world)
  if not goToWorld(CONFIG.dump_world, CONFIG.dump_world_id) then return end

  bot:drop(crop.seed_id, surplus)
  nap(2000)
end

local function runCycle(crop)
  if not goToWorld(CONFIG.world, CONFIG.world_id) then return end
  harvest(crop)
  breakBlocks(crop)
  plant(crop)
  dumpSeeds(crop)
  -- dumpSeeds may have left the bot in the storage world; the next cycle warps back.
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
bot:setAutoCollect(true)

while true do
  runCycle(crop)

  -- Wait out the growth timer, plus a random tail so cycles do not land on a
  -- fixed grid every time.
  local jitter = math.random(CONFIG.cycle_jitter_s[1], CONFIG.cycle_jitter_s[2])
  local wait = crop.grow + jitter
  log("cycle done, sleeping " .. math.floor(wait) .. "s")
  nap(wait * 1000)
end
