-- Dark Yellow Block farm loop.
--
--   harvest ready trees
--     -> when blocks hit the stack cap, place and break them back into seeds
--       -> plant every free plot
--         -> dump surplus seeds in another world
--           -> wait out the growth timer and start over
--
-- Item data comes from items.dat: Dark Yellow Block is 2018, its seed 2019,
-- grow_time 2587 seconds (43m07s), stack cap 200.
--
-- EDIT THE CONFIG BLOCK BEFORE RUNNING. The world names and the farm area are
-- the only values that must match your own world; the rest have working defaults.
--
-- Two things worth knowing before the first run:
--   * print() goes to the terminal running Mori, not the bot's web console.
--   * Breaking a block does not reliably return a seed, and often returns the
--     block itself, so the break phase is capped by max_break_rounds rather
--     than running until the blocks are gone. Watch one cycle and tune it.

local CONFIG = {
  farm_world      = "YOURFARM",   -- world the trees live in
  farm_world_id   = "",           -- door id, "" for the main entrance
  dump_world      = "YOURSTORE",  -- where surplus seeds are dropped
  dump_world_id   = "",

  block_id        = 2018,
  seed_id         = 2019,
  grow_seconds    = 2587,
  stack_cap       = 200,          -- harvesting stops here; blocks get broken
  seed_dump_at    = 50,           -- surplus seeds that trigger a dump run
  seed_keep       = 10,           -- seeds kept back after a dump

  -- Rectangle of plots to farm, inclusive, in tile coordinates.
  area            = { x1 = 10, y1 = 24, x2 = 89, y2 = 48 },

  -- Guard rails. Breaking a block does not always return a seed, so the break
  -- phase is bounded by attempts rather than by hope.
  max_break_rounds = 400,
  hits_per_block   = 12,
  step_timeout_ms  = 8000,
  action_delay_ms  = 250,
}

-- ── helpers ────────────────────────────────────────────────────────────────

local bot = getBot()

local function log(msg)
  print("[farm] " .. msg)
end

local function inv(id)
  return getInventory():findItem(id)
end

--- Walks to a tile and waits until the bot is standing on it.
--- Returns false when the bot never arrives, so callers can skip that plot.
local function goTo(x, y)
  if bot:isInTile(x, y) then return true end
  bot:findPath(x, y)

  local waited = 0
  while waited < CONFIG.step_timeout_ms do
    if bot:isInTile(x, y) then return true end
    sleep(100)
    waited = waited + 100
  end
  return false
end

local function inFarmWorld()
  return bot:isInWorld(CONFIG.farm_world)
end

--- Warps and waits for the world to load. Returns false if it never arrives.
local function goToWorld(name, id)
  if bot:isInWorld(name) then return true end
  log("warping to " .. name)
  bot:warp(name, id)

  local waited = 0
  while waited < CONFIG.step_timeout_ms do
    if bot:isInWorld(name) then
      sleep(1000) -- let the map finish loading before reading tiles
      return true
    end
    sleep(250)
    waited = waited + 250
  end
  log("warp to " .. name .. " failed")
  return false
end

--- Tiles inside the configured area that satisfy `pred(tile)`.
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

--- Any ready-to-harvest tree inside the area. Deliberately not filtered by item
--- id: in a dedicated farm world everything ready is worth picking, and it keeps
--- the script working if you switch crops.
local function readyTrees()
  return plots(function(t) return t:canHarvest() end)
end

local function emptyPlots()
  return plots(function(t) return t.fg == 0 end)
end

--- Punches a tile until it clears, or until the hit budget runs out. Used for
--- both harvesting a tree and breaking a placed block.
local function punchUntilClear(x, y)
  for _ = 1, CONFIG.hits_per_block do
    bot:hit(x, y)
    sleep(CONFIG.action_delay_ms)

    local tile = getTile(x, y)
    if tile and tile.fg == 0 then return true end
  end
  return false
end

-- ── phases ─────────────────────────────────────────────────────────────────

--- Harvests ready trees until none are left or the block stack fills up.
local function harvest()
  local trees = readyTrees()
  log("harvest: " .. #trees .. " trees ready, " .. inv(CONFIG.block_id) .. " blocks held")

  for _, plot in ipairs(trees) do
    if inv(CONFIG.block_id) >= CONFIG.stack_cap then
      log("harvest: stack cap reached, switching to break phase")
      return
    end
    if not inFarmWorld() then return end

    if goTo(plot.x, plot.y) then
      punchUntilClear(plot.x, plot.y)
    end
  end
end

--- Places blocks back down and breaks them, which is what yields seeds.
--- Bounded by max_break_rounds: a block does not always drop a seed, and a
--- broken block often comes straight back, so this can never be "until done".
local function breakBlocks()
  local rounds = 0

  while inv(CONFIG.block_id) > 0 and rounds < CONFIG.max_break_rounds do
    if not inFarmWorld() then return end

    local free = emptyPlots()
    if #free == 0 then
      log("break: no free plot to place on")
      return
    end

    local plot = free[math.random(#free)]
    if goTo(plot.x, plot.y) then
      bot:place(plot.x, plot.y, CONFIG.block_id)
      sleep(CONFIG.action_delay_ms)
      punchUntilClear(plot.x, plot.y)
      -- Auto-collect picks the drops up; give it a tick to do so.
      sleep(CONFIG.action_delay_ms)
    end
    rounds = rounds + 1
  end

  log("break: done after " .. rounds .. " rounds, "
    .. inv(CONFIG.seed_id) .. " seeds held")
end

--- Plants one seed on every free plot, until the seeds run out.
local function plant()
  local free = emptyPlots()
  log("plant: " .. #free .. " free plots, " .. inv(CONFIG.seed_id) .. " seeds held")

  for _, plot in ipairs(free) do
    if inv(CONFIG.seed_id) <= 0 then return end
    if not inFarmWorld() then return end

    if goTo(plot.x, plot.y) then
      bot:place(plot.x, plot.y, CONFIG.seed_id)
      sleep(CONFIG.action_delay_ms)
    end
  end
end

--- Drops surplus seeds in the storage world, keeping `seed_keep` back.
local function dumpSeeds()
  local surplus = inv(CONFIG.seed_id) - CONFIG.seed_keep
  if inv(CONFIG.seed_id) < CONFIG.seed_dump_at or surplus <= 0 then return end

  log("dump: " .. surplus .. " seeds to " .. CONFIG.dump_world)
  if not goToWorld(CONFIG.dump_world, CONFIG.dump_world_id) then return end

  bot:drop(CONFIG.seed_id, surplus)
  sleep(2000)
  goToWorld(CONFIG.farm_world, CONFIG.farm_world_id)
end

-- ── main loop ──────────────────────────────────────────────────────────────

math.randomseed(inv(CONFIG.seed_id) + inv(CONFIG.block_id) + 1)
bot:setAutoCollect(true)

while true do
  if goToWorld(CONFIG.farm_world, CONFIG.farm_world_id) then
    harvest()
    breakBlocks()
    plant()
    dumpSeeds()
  end

  -- Wait out the growth timer, plus a random tail so every cycle starts at a
  -- slightly different time of day rather than on a fixed 43m07s grid.
  local wait = CONFIG.grow_seconds + math.random(30, 300)
  log("cycle done, sleeping " .. wait .. "s")
  sleep(wait * 1000)
end
