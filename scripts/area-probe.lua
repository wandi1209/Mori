-- Area probe: prints what a candidate farm area actually contains.
--
-- Run this before farm.lua, with the bot standing in the farm world. It walks
-- nothing and changes nothing — it only reads the world snapshot and reports how
-- many plots, ready trees and solid tiles fall inside each area you list, so the
-- numbers in farm.lua's CONFIG can be checked instead of guessed.
--
-- Output goes to the terminal running Mori (docker compose logs -f).

local AREAS = {
  { name = "candidate",   x1 = 2,  y1 = 2,  x2 = 99, y2 = 4  },
  { name = "whole world", x1 = 0,  y1 = 0,  x2 = 99, y2 = 59 },
}

local world = getWorld()
if not world then
  print("[probe] not in a world")
  return
end

-- World.x and World.y are the map's width and height, not a position.
local width, height = world.x, world.y
print(("[probe] world %s, %dx%d"):format(world.name, width, height))

for _, a in ipairs(AREAS) do
  local empty, ready, growing, solid = 0, 0, 0, 0

  for _, t in ipairs(getTiles()) do
    if t.x >= a.x1 and t.x <= a.x2 and t.y >= a.y1 and t.y <= a.y2 then
      if t.fg == 0 then
        empty = empty + 1
      elseif t:canHarvest() then
        ready = ready + 1
      elseif t:hasExtra() and t:getExtra() and t:getExtra().type == "seed" then
        growing = growing + 1
      else
        solid = solid + 1
      end
    end
  end

  print(("[probe] %-12s x %d..%d y %d..%d -> %d empty, %d ready, %d growing, %d solid")
    :format(a.name, a.x1, a.x2, a.y1, a.y2, empty, ready, growing, solid))
end

-- Rows are easier to place an area around when you can see where the plots are.
print("[probe] rows with plots or trees:")
local rows = {}
for _, t in ipairs(getTiles()) do
  local seedish = t.fg == 0 or t:canHarvest()
  if seedish then
    rows[t.y] = (rows[t.y] or 0) + 1
  end
end
for y = 0, height - 1 do
  if rows[y] and rows[y] > 10 then
    print(("[probe]   row %2d: %d empty-or-ready tiles"):format(y, rows[y]))
  end
end
