base = "attack"
-- Corrupt through PoB's window: implicits on a rare amulet, then roll ranges on a unique, and
-- the window refused for an item that is already corrupted. Item text is literal. Picks are
-- made by label, and the list is read when the pick is made: PoB builds Corrupt's implicit
-- lists from `pairs`, so tied entries sit in another order per Lua state.
local AMULET = [[Rarity: RARE
Doom Pendant
Onyx Amulet
Implicits: 1
{tags:attribute}+12 to all Attributes
+70 to maximum Life
+30% to Fire Resistance]]
local UNIQUE = [[Rarity: UNIQUE
Kaom's Test
Glorious Plate
Implicits: 0
+(400-500) to maximum Life
+(10-20)% to Fire Resistance]]

local function labelOf(entry)
	return type(entry) == "table" and entry.label or entry
end

local function pick(name, index)
	local control = main.popups[1].controls[name]
	local entry = control.list[index + 1]
	assert(entry ~= nil, name .. " has no entry " .. index)
	act.popup_set(name, { Choice = { index = index, label = labelOf(entry) } })
end

function recipe()
	local tab = build.itemsTab
	act.paste(AMULET)
	act.open_popup("displayItemCorrupt")
	pick("implicit1", 1)
	pick("implicit2", 2)
	act.popup_press("save")
	assert(tab.displayItem.corrupted, "not corrupted")
	checkpoint()
	act.save()

	-- roll ranges on a unique
	act.paste(UNIQUE)
	act.open_popup("displayItemCorrupt")
	act.popup_press("rolls")
	local first
	for key in pairs(main.popups[1].controls) do
		local n = type(key) == "string" and tonumber(key:match("^rollRangeSlider(%d+)$"))
		if n and (not first or n < first) then
			first = n
		end
	end
	assert(first, "no roll range slider")
	local slider = "rollRangeSlider" .. first
	act.popup_set(slider, { Number = 1.0 })
	act.popup_press("save")
	checkpoint()

	-- corrupt again: whether PoB still offers the window is part of the checkpoint (the
	-- display item lists the windows it offers); when it does, it opens and closes again
	if tab.controls.displayItemCorrupt:IsShown() then
		act.open_popup("displayItemCorrupt")
		checkpoint()
		act.popup_close()
	end
	checkpoint()
	act.save()
end
