base = "attack"
-- Custom, implicit and crucible mods through PoB's windows: a bench craft, an essence and a
-- custom line, removing one, an Exarch implicit, then a crucible mod on a sword. Item text is
-- literal. Picks are made by label, and the list is read when the pick is made: PoB builds the
-- Essence and Exarch lists from `pairs`, so tied entries sit in another order per Lua state.
local BODY = [[Rarity: RARE
Doom Plate
Glorious Plate
Searing Exarch Item
Implicits: 0
+50 to maximum Life]]
local SWORD = [[Rarity: RARE
Doom Edge
Rusted Sword
Implicits: 0
+10 to Strength]]

local function labelOf(entry)
	return type(entry) == "table" and entry.label or entry
end

-- Picks entry `index` (0-based) of a dropdown of the open window, recording its label
local function pick(name, index)
	local control = main.popups[1].controls[name]
	local entry = control.list[index + 1]
	assert(entry ~= nil, name .. " has no entry " .. index)
	act.popup_set(name, { Choice = { index = index, label = labelOf(entry) } })
end

-- Picks the entry of a dropdown whose label is `label`
local function pickLabel(name, label)
	local index, full = entryIndex(main.popups[1].controls[name], label)
	act.popup_set(name, { Choice = { index = index, label = full } })
end

function recipe()
	local tab = build.itemsTab
	act.paste(BODY)
	assert(tab.displayItem.rarity == "RARE" and tab.displayItem.cleansing, "no Exarch rare")
	assert(#displayDump().custom_mods == 0, "the plate starts with custom mods")

	-- the crafting bench, first modifier
	act.open_popup("displayItemAddCustom")
	pickLabel("source", "Crafting Bench")
	pick("modSelect", 0)
	act.popup_press("save")

	-- an essence: the third entry
	act.open_popup("displayItemAddCustom")
	pickLabel("source", "Essence")
	pick("modSelect", 2)
	act.popup_press("save")

	-- a line of your own
	act.open_popup("displayItemAddCustom")
	pickLabel("source", "Custom")
	act.popup_set("custom", { Text = "+25 to Strength" })
	act.popup_press("save")
	local dump = displayDump()
	assert(#dump.custom_mods == 3, "three custom mods expected, got " .. #dump.custom_mods)
	local essence = dump.custom_mods[2].line
	assert(dump.custom_mods[1].kind == "Crafted" and dump.custom_mods[2].kind == "Custom"
		and dump.custom_mods[3].kind == "Custom", "the kinds")
	assert(dump.custom_mods[3].line:find("+25 to Strength", 1, true), dump.custom_mods[3].line)
	checkpoint()

	-- the essence goes again
	act.remove_custom(1)
	dump = displayDump()
	assert(#dump.custom_mods == 2 and dump.custom_mods[2].kind == "Custom", "the essence is still there")
	assert(dump.custom_mods[1].kind == "Crafted" and dump.custom_mods[2].line ~= essence, "the wrong line went")
	checkpoint()

	-- an Exarch implicit: the second type, its first modifier
	act.open_popup("displayItemAddImplicit")
	pickLabel("source", "Searing Exarch")
	pick("modGroupSelect", 1)
	pick("modSelect", 0)
	act.popup_press("save")
	checkpoint()
	act.save()

	-- a crucible mod on a sword: node 1's second entry
	act.paste(SWORD)
	act.open_popup("displayItemAddCrucible")
	pick("modSelectNode1", 1)
	act.popup_press("save")
	dump = displayDump()
	assert(#dump.custom_mods == 1 and dump.custom_mods[1].kind == "Crucible", "no crucible mod")
	checkpoint()
	act.save()
end
