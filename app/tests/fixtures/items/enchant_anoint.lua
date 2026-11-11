base = "attack"
-- Enchant, remove, then anoint through PoB's windows: a helmet is enchanted for the build's main
-- skill, enchanted again and stripped; an amulet is anointed after a search and a stat sort.
-- Item text is literal. Picks are made by label.
local HELMET = [[Rarity: RARE
Doom Crown
Iron Hat
Implicits: 0
+60 to maximum Life]]
local AMULET = [[Rarity: RARE
Doom Pendant
Onyx Amulet
Implicits: 1
{tags:attribute}+12 to all Attributes
+70 to maximum Life
+30% to Fire Resistance]]

-- Picks entry `index` (0-based) of a dropdown control, recording its label
local function pick(control, name, index)
	local entry = control.list[index + 1]
	assert(entry ~= nil, name .. " has no entry " .. index)
	act.popup_set(name, { Choice = { index = index, label = type(entry) == "table" and entry.label or entry } })
end

-- Picks the entry of a dropdown control whose label is `label`
local function pickLabel(control, name, label)
	local index, full = entryIndex(control, label)
	act.popup_set(name, { Choice = { index = index, label = full } })
end

function recipe()
	local tab = build.itemsTab
	act.paste(HELMET)
	act.open_popup("displayItemEnchant")
	local controls = main.popups[1].controls
	pickLabel(controls.skill, "skill", "Boneshatter")
	pick(controls.enchantment, "enchantment", 1)
	act.popup_press("save")
	assert(#tab.displayItem.enchantModLines == 1, "no enchantment")
	checkpoint()

	-- again, then take it off
	act.open_popup("displayItemEnchant")
	controls = main.popups[1].controls
	pickLabel(controls.skill, "skill", "Boneshatter")
	pick(controls.enchantment, "enchantment", 0)
	act.popup_press("remove")
	assert(#tab.displayItem.enchantModLines == 0, "the enchantment stayed")
	checkpoint()

	-- anoint: search, sort by Life, take the first notable
	act.paste(AMULET)
	act.open_popup("displayItemAnoint")
	local list = main.popups[1].controls.notableDB
	act.popup_set("notableDB.search", { Text = "Constitution" })
	assert(#list.list >= 1, "Constitution matches no notable")
	checkpoint()

	pickLabel(list.controls.sort, "notableDB.sort", "Sort by Life")
	checkpoint()

	local key = popupDump().controls
	local rows
	for _, c in ipairs(key) do
		if c.name == "notableDB" then rows = c.rows end
	end
	assert(rows and #rows > 0, "the sorted list is empty")
	act.popup_set("notableDB", { Row = rows[1].key })
	checkpoint()
	act.popup_press("save")
	assert(#tab.displayItem.enchantModLines >= 1, "no anointment")
	checkpoint()
	act.save()
end
