base = "attack"
-- Craft a rare ring with PoB's crafter: three prefixes with rolls and three suffixes, then the
-- modifier sorting, which re-orders every list.
local function lastIndex(slot)
	return #build.itemsTab.controls["displayItemAffix" .. (slot + 1)].list - 1
end

local function labels(slot)
	local out = {}
	for i, entry in ipairs(build.itemsTab.controls["displayItemAffix" .. (slot + 1)].list) do
		out[i] = type(entry) == "table" and entry.label or entry
	end
	return table.concat(out, "\n")
end

function recipe()
	local tab = build.itemsTab
	assert(tab.displayItem == nil, "an item is displayed")
	craftBase("Rare", "Gate Ring", "Ring", "Two-Stone Ring")
	local item = tab.displayItem
	assert(item and item.crafted and item.title == "Gate Ring" and item.baseName == "Two-Stone Ring", "no crafted ring")
	local dump = displayDump()
	assert(#dump.affixes == 6, "a ring offers six affixes, not " .. #dump.affixes)
	local offered = false
	for _, kind in ipairs(dump.popups) do offered = offered or kind == "AddModifier" end
	assert(offered, "Add modifier is not offered")
	assert(dump.crafting_sort, "no modifier sorting")
	assert(dump.cluster == nil, "a ring has no cluster controls")
	checkpoint()

	-- the 2nd, 5th and last entry of each list as it stands when the pick is made
	local ranges = { [0] = 0.25, [1] = 0.5, [2] = 1.0 }
	local picks = { 1, 4, "last", 1, 4, "last" }
	for slot = 0, 5 do
		local index = picks[slot + 1]
		if index == "last" then index = lastIndex(slot) end
		act.affix(slot, index)
		if ranges[slot] then act.affix_range(slot, ranges[slot]) end
	end
	local ranged = 0
	for _, affix in ipairs(displayDump().affixes) do
		assert(affix.list.selected > 0, "slot " .. affix.slot .. " is empty")
		if affix.range then ranged = ranged + 1 end
	end
	assert(ranged >= 3, "the prefixes have no sliders")
	checkpoint()

	-- the stat sort (Open check 6): the lists come out in another order
	local before = labels(0)
	local sortIndex = entryIndex(tab.controls.craftingSorting, "Life")
	local started = os.clock()
	act.crafting_sort(sortIndex)
	io.stderr:write(string.format("upstream: Sort by Life took %.3f s\n", os.clock() - started))
	assert(labels(0) ~= before, "the sort left slot 0's list as it was")
	assert(tab.selControl == nil, "selControl is set upstream")
	checkpoint()

	-- slot 0 again, under the new order
	local drop = tab.controls.displayItemAffix1
	act.affix(0, drop.selIndex == 3 and 2 or 3)
	checkpoint()

	act.save()
	local equipped
	for _, name in ipairs({ "Ring 1", "Ring 2" }) do
		local slot = tab.slots[name]
		if slot.selItemId ~= 0 and tab.items[slot.selItemId].title == "Gate Ring" then equipped = name end
	end
	assert(equipped, "the ring was not equipped")
	checkpoint()
end
