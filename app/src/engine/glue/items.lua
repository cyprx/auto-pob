-- The Items tab's state, as the tab's controls would show it.
-- PoB syncs some of the tab's controls only while it draws (the item list, the socket labels
-- and visibility), which never runs here: the tab is never on screen, because `frame()` pins
-- the tree tab. A view runs those same updates itself and then only reads. It never calls
-- `PopulateSlots`, which unequips what no longer fits, and it adds no undo state.

glue = glue or {}

local function items_tab()
	return pob.build.itemsTab
end

-- The slot's kind, in the order the tab's controls tell them apart
local function slot_kind(slot)
	if slot.nodeId then
		return "Jewel"
	elseif slot.parentSlot then
		return "Abyssal"
	elseif slot.controls.activate then
		return "Flask"
	elseif slot.slotName:match("^Graft") then
		return "Graft"
	elseif slot.weaponSet == 2 then
		return "Swap"
	end
	return "Gear"
end

local function slot_view(slot)
	local view = {
		name = slot.slotName,
		label = slot.label,
		kind = slot_kind(slot),
		parent = slot.parentSlot and slot.parentSlot.slotName or nil,
		node = slot.nodeId,
		shown = slot:IsShown() and true or false,
		enabled = slot:IsEnabled() and true or false,
		selected = slot.selItemId,
		choices = {},
	}
	-- the dropdown's first entry is "None"
	for i = 2, #slot.items do
		view.choices[i - 1] = slot.items[i]
	end
	if slot.controls.activate then
		view.flask_active = slot.active and true or false
	end
	return view
end

local function item_entry(tab, list, index, id)
	local item = tab.items[id]
	local entry = {
		id = id,
		name = item.name,
		base = item.baseName or "",
		rarity = item.rarity,
		colour = colorCodes[item.rarity] or "",
		display_label = list:GetRowValue(1, index, id),
		primary_slot = item:GetPrimarySlot(),
		abyss_set = list:FindEquippedAbyssJewel(id, true),
		tree = list:FindSocketedJewel(id, true),
	}
	local slot, set = tab:GetEquippedSlotForItem(item)
	if slot then
		entry.equipped = {
			slot = slot.slotName,
			label = slot.label,
			set = set and (set.title or "Default") or nil,
		}
	end
	return entry
end

function glue.items_view()
	local tab = items_tab()
	local list = tab.controls.itemList
	-- what ItemListControl:Draw and ItemsTab:Draw do before they draw (R16)
	list:UpdateList()
	tab:UpdateSockets()
	local view = {
		items = {},
		list_filter = glue.drop_list(list.controls.loadoutFilter),
		listed = {},
		can_reorder = list.isMutable and true or false,
		slots = {},
		sets = {},
		active_set = tab.activeItemSetId,
		second_weapon_set = tab.activeItemSet.useSecondWeaponSet and true or false,
		show_stat_differences = tab.showStatDifferences and true or false,
		can_undo = tab.undo[2] ~= nil,
		can_redo = tab.redo[1] ~= nil,
	}
	for i, id in ipairs(tab.itemOrderList) do
		view.items[i] = item_entry(tab, list, i, id)
	end
	for i, id in ipairs(list.list) do
		view.listed[i] = id
	end
	for i, slot in ipairs(tab.orderedSlots) do
		view.slots[i] = slot_view(slot)
	end
	for i, id in ipairs(tab.itemSetOrderList) do
		view.sets[i] = { id = id, title = tab.itemSets[id].title or "Default" }
	end
	return view
end
