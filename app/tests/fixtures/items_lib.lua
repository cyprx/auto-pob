-- Helpers for the Items tab tests. Runs in unmodified upstream PoB after a build is loaded.
-- Written independently of glue/items.lua: values are read from the controls after the
-- updates PoB's own Draw runs (the item list's `UpdateList`, `UpdateSockets`), and slot kinds
-- come from the slot names. Nothing here may call our glue.

local function tab()
	return build.itemsTab
end

local function slotKind(slot)
	local name = slot.slotName
	if slot.nodeId then return "Jewel" end
	if name:find("Abyssal Socket", 1, true) then return "Abyssal" end
	if name:find("^Flask") then return "Flask" end
	if name:find("^Graft") then return "Graft" end
	if name:find(" Swap$") then return "Swap" end
	return "Gear"
end

-- The Items tab as plain tables
function itemsDump()
	local t = tab()
	local list = t.controls.itemList
	local filter = list.controls.loadoutFilter
	-- what the tab does before it draws
	list:UpdateList()
	t:UpdateSockets()

	local out = {
		items = {}, slots = {}, sets = {}, listed = {},
		list_filter = { labels = {}, selected = filter.selIndex - 1, enabled = filter:IsEnabled() and true or false },
		can_reorder = list.isMutable and true or false,
		active_set = t.activeItemSetId,
		second_weapon_set = t.activeItemSet.useSecondWeaponSet and true or false,
		show_stat_differences = t.showStatDifferences and true or false,
		can_undo = t.undo[2] ~= nil,
		can_redo = t.redo[1] ~= nil,
	}
	for i, label in ipairs(filter.list) do out.list_filter.labels[i] = label end
	for i, id in ipairs(list.list) do out.listed[i] = id end

	for i, id in ipairs(t.itemOrderList) do
		local item = t.items[id]
		local entry = {
			id = id,
			name = item.name,
			base = item.baseName or "",
			rarity = item.rarity,
			colour = colorCodes[item.rarity],
			display_label = list:GetRowValue(1, i, id),
			primary_slot = item:GetPrimarySlot(),
			abyss_set = list:FindEquippedAbyssJewel(id, true),
			tree = list:FindSocketedJewel(id, true),
		}
		local slot, set = t:GetEquippedSlotForItem(item)
		if slot then
			entry.equipped = { slot = slot.slotName, label = slot.label, set = set and (set.title or "Default") or nil }
		end
		out.items[i] = entry
	end

	for i, slot in ipairs(t.orderedSlots) do
		local entry = {
			name = slot.slotName,
			label = slot.label,
			kind = slotKind(slot),
			parent = slot.parentSlot and slot.parentSlot.slotName or nil,
			node = slot.nodeId,
			shown = slot:IsShown() and true or false,
			enabled = slot:IsEnabled() and true or false,
			selected = slot.selItemId,
			choices = {},
		}
		for j, id in ipairs(slot.items) do
			if id ~= 0 then table.insert(entry.choices, id) end
		end
		if slot.controls.activate then
			entry.flask_active = slot.controls.activate.state and true or false
		end
		out.slots[i] = entry
	end

	for i, id in ipairs(t.itemSetOrderList) do
		out.sets[i] = { id = id, title = t.itemSets[id].title or "Default" }
	end
	return out
end
