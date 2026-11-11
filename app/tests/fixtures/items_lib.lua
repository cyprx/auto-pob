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

-- ---- Tooltips ----

-- A PoB Tooltip as plain tables. Separators are the lines with no text. Written apart from the
-- glue's reader: it takes the header fields as they are and reads each line's fields by name.
function tooltipDump(tt)
	local out = { influences = {}, lines = {} }
	if tt.tooltipHeader then out.header = tostring(tt.tooltipHeader) end
	if tt.influenceHeader1 then out.influences[#out.influences + 1] = tt.influenceHeader1 end
	if tt.influenceHeader2 then out.influences[#out.influences + 1] = tt.influenceHeader2 end
	out.foil = tt.foilType or nil
	if type(tt.color) == "string" then out.colour = tt.color end
	for i, line in ipairs(tt.lines) do
		if line.text == nil then
			out.lines[i] = { separator = line.size }
		else
			out.lines[i] = { size = line.size, text = line.text, font = line.font, center = line.center or false }
			if type(line.background) == "string" then out.lines[i].background = line.background end
		end
	end
	return out
end

-- Hovering a row of the item list: its own tooltip hook, on a fresh Tooltip
function listTooltip(id)
	local list = tab().controls.itemList
	list:UpdateList()
	tab():UpdateSockets()
	for i, listed in ipairs(list.list) do
		if listed == id then
			local tt = new("Tooltip")
			list:AddValueTooltip(tt, i, id)
			return tooltipDump(tt)
		end
	end
end

-- Hovering a slot's dropdown entry for `id`: the slot's own tooltip hook
function slotTooltip(slotName, id)
	local slot = tab().slots[slotName]
	tab():UpdateSockets()
	for i, entry in ipairs(slot.items) do
		if entry == id then
			local tt = new("Tooltip")
			slot.tooltipFunc(tt, "HOVER", i, id)
			return tooltipDump(tt)
		end
	end
end

-- The Copy hook of the list, with `Copy` captured for the call and put back
function copyItem(id)
	local list = tab().controls.itemList
	local saved, text = Copy, nil
	Copy = function(copied) text = copied end
	for i, listed in ipairs(tab().itemOrderList) do
		if listed == id then list:OnSelCopy(i, id) end
	end
	Copy = saved
	return text
end

-- ---- Recipe actions (R20) ----
-- Every `act.*` presses PoB's own control the way the tab does, records the command for the
-- gate to replay (`cmd` is our command's name, positions are 0-based), then runs a frame.
-- Nothing here may call our glue. `act.alloc` and `act.dealloc` are tree_lib's, wrapped to
-- record in this shape; the host defines `sidebar_lines()`.

actions = {}
act = act or {}
pick = pick or {}

local function frame()
	runCallback("OnFrame")
end

local function record(entry)
	table.insert(actions, entry)
	frame()
end

local treeAlloc, treeDealloc = act.alloc, act.dealloc

function act.alloc(node)
	treeAlloc(node)
	actions[#actions] = { cmd = "AllocNode", id = node.id }
end

function act.dealloc(node)
	treeDealloc(node)
	actions[#actions] = { cmd = "DeallocNode", id = node.id }
end

local function itemList()
	local list = tab().controls.itemList
	list:UpdateList()
	return list
end

-- Presses the `confirm` button of the newest popup
local function confirm()
	local popup = main.popups[1]
	assert(popup, "no popup is open")
	popup.controls.confirm.onClick()
end

-- The id of the item with this name (the item's name also holds its base, "Name, Base")
function pick.item(name)
	for id, item in pairs(tab().items) do
		if item.name == name or item.name:sub(1, #name + 1) == name .. "," then return id end
	end
	error("no item " .. name)
end

-- The Sort button (ItemListControl.lua:20-23)
function act.sort()
	itemList().controls.sort.onClick()
	record({ cmd = "SortItems" })
end

-- A click on the row, then the Delete button and its confirmation if one opens
-- (ItemListControl.lua:75-78, 242-258)
function act.delete(id)
	local list = itemList()
	local found
	for i, listed in ipairs(list.list) do
		if listed == id then found = i end
	end
	assert(found, "item " .. tostring(id) .. " is not listed")
	list.selIndex, list.selValue = found, id
	local depth = #main.popups
	list.controls.delete.onClick()
	if #main.popups > depth then confirm() end
	record({ cmd = "DeleteItem", id = id })
end

-- The Del Unused button (ItemListControl.lua:24-46)
function act.delete_unused()
	itemList().controls.deleteUnused.onClick()
	record({ cmd = "DeleteUnusedItems" })
end

-- The Del All button and its confirmation (ItemListControl.lua:47-68)
function act.delete_all()
	local list = itemList()
	assert(#list.list > 0, "the list is empty")
	list.controls.deleteAll.onClick()
	confirm()
	record({ cmd = "DeleteAllItems" })
end

-- Dragging the row to a new place and dropping it (ListControl.lua:426-436); `to` is the
-- final position, which a drop expresses as the row it lands before.
function act.move(from, to)
	local list = itemList()
	assert(list.isMutable, "the list is filtered")
	list.selIndex, list.selValue = from + 1, list.list[from + 1]
	local dragIndex = to > from and to + 2 or to + 1
	table.remove(list.list, list.selIndex)
	if dragIndex > list.selIndex then
		dragIndex = dragIndex - 1
	end
	table.insert(list.list, dragIndex, list.selValue)
	list:OnOrderChange(list.selIndex, dragIndex)
	list.selValue = nil
	record({ cmd = "MoveItem", from = from, to = to })
end

-- The loadout filter dropdown (ItemListControl.lua:15-17)
function act.filter(index)
	local list = itemList()
	list.controls.loadoutFilter:SetSel(index + 1)
	record({ cmd = "SetItemListFilter", index = index })
end

-- The slot's dropdown: its first entry is "None" (ItemSlotControl.lua:12-19)
function act.equip(slotName, id)
	local t = tab()
	t:UpdateSockets()
	local slot = t.slots[slotName]
	local found = 1
	if id ~= nil then
		found = nil
		for i, entry in ipairs(slot.items) do
			if entry == id then found = i end
		end
	end
	assert(found, "item " .. tostring(id) .. " is not in the dropdown of " .. slotName)
	-- the dropdown is on a different row than it was drawn on: put it on the selected one
	slot.selIndex = 1
	for i, entry in ipairs(slot.items) do
		if entry == slot.selItemId then slot.selIndex = i end
	end
	slot:SetSel(found)
	record({ cmd = "EquipItem", slot = slotName, id = id })
end

-- A flask's own checkbox (ItemSlotControl.lua:22-30)
function act.flask(slotName, on)
	local slot = tab().slots[slotName]
	local activate = slot.controls.activate
	assert(activate:IsEnabled(), "the flask slot is empty")
	if on ~= (slot.active and true or false) then
		activate.state = on
		activate.changeFunc(on)
	end
	record({ cmd = "SetFlaskActive", slot = slotName, on = on })
end

-- The "I" and "II" buttons (ItemsTab.lua:224-263)
function act.weapon_set(second)
	local c = tab().controls
	local button = second and c.weaponSwap2 or c.weaponSwap1
	if not button.locked() then button.onClick() end
	record({ cmd = "SelectWeaponSet", second = second })
end

-- Ctrl+Z and Ctrl+Y on the tab (ItemsTab.lua:1458-1465)
function act.undo()
	tab():Undo()
	build.buildFlag = true
	record({ cmd = "Undo" })
end

function act.redo()
	tab():Redo()
	build.buildFlag = true
	record({ cmd = "Redo" })
end

-- The Items tab right now, recorded for the gate to compare with ours at this point
function checkpoint()
	frame()
	local jewels = {}
	for node, id in pairs(build.spec.jewels) do
		if id ~= 0 then jewels[node] = id end
	end
	table.insert(actions, { check = { items = itemsDump(), jewels = jewels, sidebar = sidebar_lines() } })
end
