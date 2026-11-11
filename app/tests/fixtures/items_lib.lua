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

-- ---- The display item ----

local rarityIndex = { NORMAL = 0, MAGIC = 1, RARE = 2, UNIQUE = 3, RELIC = 4 }

-- The kinds of mod line a tooltip line can toggle, by the item's list
local modLineLists = {
	{ "enchant", "enchantModLines" }, { "scourge", "scourgeModLines" },
	{ "implicit", "implicitModLines" }, { "explicit", "explicitModLines" },
	{ "crucible", "crucibleModLines" },
}

local function modLineRef(item, modLine)
	for _, entry in ipairs(modLineLists) do
		for i = 1, #item[entry[2]] do
			if item[entry[2]][i] == modLine then return { list = entry[1], index = i - 1 } end
		end
	end
end

local function dropDump(control)
	local labels = {}
	for i, entry in ipairs(control.list) do
		labels[i] = type(entry) == "table" and entry.label or entry
	end
	return { labels = labels, selected = control.selIndex - 1, enabled = control:IsEnabled() and true or false }
end

-- The display panel as plain tables, or nil without a displayed item. The tooltip is rebuilt
-- first, as PoB does when an item is edited (R4); the sockets are read from their controls
-- after the update PoB makes when it displays an item.
function displayDump()
	local t = tab()
	local item = t.displayItem
	if not item then return nil end
	t:UpdateDisplayItemTooltip()
	t:UpdateSocketControls()
	local c = t.controls
	local out = {
		id = item.id,
		name = item.name,
		base = item.baseName or "",
		rarity = item.rarity,
		edit_rarity = rarityIndex[item.rarity],
		edit_text = (item:BuildRaw():gsub("Rarity: %w+\n", "")),
		save_label = t.items[item.id] and "Save" or "Add to build",
		variants = {}, sockets = {}, influences = {}, range_lines = {},
	}
	if c.displayItemVersion:IsShown() then out.version = dropDump(c.displayItemVersion) end
	for i, name in ipairs({ "displayItemVariant", "displayItemAltVariant", "displayItemAltVariant2",
		"displayItemAltVariant3", "displayItemAltVariant4", "displayItemAltVariant5" }) do
		if c[name]:IsShown() then
			table.insert(out.variants, { control = i - 1, list = dropDump(c[name]) })
		end
	end
	local count = #item.sockets - item.abyssalSocketCount
	for i = 1, math.min(count, 6) do
		local drop = c["displayItemSocket" .. i]
		local link = c["displayItemLink" .. i]
		local linked
		if count > i then linked = link.state and true or false end
		out.sockets[i] = {
			colour = drop.list[drop.selIndex].color,
			shown = drop:IsShown() and true or false,
			linked = linked,
		}
	end
	out.can_add_socket = c.displayItemAddSocket:IsShown() and true or false
	if c.displayItemInfluence:IsShown() then
		out.influences = { dropDump(c.displayItemInfluence), dropDump(c.displayItemInfluence2) }
	end
	if c.displayItemQualityEdit:IsShown() then out.quality = c.displayItemQualityEdit.buf end
	if c.displayItemCatalyst:IsShown() then out.catalyst = dropDump(c.displayItemCatalyst) end
	if c.displayItemCatalystQualityEdit:IsShown() then out.catalyst_quality = c.displayItemCatalystQualityEdit.buf end
	for i, modLine in ipairs(item.rangeLineList) do
		out.range_lines[i] = {
			label = modLine.line, range = modLine.range, slider = modLine.showSlider and true or false,
			mutable = (modLine.modId and modLine.newModId) and true or false, mutated = modLine.mutated and true or false,
		}
	end
	out.popups = {}
	for _, opener in ipairs({
		{ "displayItemEnchant", { Enchant = { second = false } } },
		{ "displayItemEnchant2", { Enchant = { second = true } } },
		{ "displayItemAnoint", { Anoint = { slot = 0 } } },
		{ "displayItemAnoint2", { Anoint = { slot = 1 } } },
		{ "displayItemAnoint3", { Anoint = { slot = 2 } } },
		{ "displayItemAnoint4", { Anoint = { slot = 3 } } },
		{ "displayItemCorrupt", "Corrupt" },
		{ "displayItemAddImplicit", "AddImplicit" },
		{ "displayItemAddCustom", "AddModifier" },
		{ "displayItemAddCrucible", "AddCrucible" },
	}) do
		if c[opener[1]]:IsShown() then table.insert(out.popups, opener[2]) end
	end
	-- the crafter's dropdowns, from the controls PoB keeps up to date (UpdateAffixControls)
	out.affixes = {}
	for i = 1, 9 do
		local drop = c["displayItemAffix" .. i]
		if drop:IsShown() then
			local affix = {
				slot = i - 1,
				label = (c["displayItemAffixLabel" .. i]:GetProperty("label"):gsub("^%^7", "")),
				list = dropDump(drop),
			}
			if drop.slider:IsShown() then
				affix.range = drop.slider.val
				affix.tiers = drop.slider.divCount
			end
			out.affixes[#out.affixes + 1] = affix
		end
	end
	if c.craftingSorting:IsShown() then out.crafting_sort = dropDump(c.craftingSorting) end
	if c.displayItemClusterJewelSkill:IsShown() then
		out.cluster = {
			skill = dropDump(c.displayItemClusterJewelSkill),
			count = item.clusterJewelNodeCount,
			min = item.clusterJewel.minNodes,
			max = item.clusterJewel.maxNodes,
		}
	end
	out.tooltip = tooltipDump(t.displayItemTooltip)
	for i, line in ipairs(t.displayItemTooltip.lines) do
		if line.modLine then out.tooltip.lines[i].mod_line = modLineRef(item, line.modLine) end
	end
	return out
end

-- Hovering an entry of affix dropdown `slot` (0..8; `index` is 0-based) or, without an index,
-- the slider under it: the control's own tooltip hook, on a fresh Tooltip
function affixTooltip(slot, index)
	local drop = tab().controls["displayItemAffix" .. (slot + 1)]
	if not (drop and drop:IsShown()) then return nil end
	local tt = new("Tooltip")
	if index then
		local entry = drop.list[index + 1]
		if entry == nil then return nil end
		drop.tooltipFunc(tt, "HOVER", index + 1, entry)
	else
		if not drop.slider:IsShown() then return nil end
		drop.slider.tooltipFunc(tt, drop.slider.val)
	end
	return tooltipDump(tt)
end

-- The affix tooltips a checkpoint compares: slot 0's first, middle and last entry, then its
-- slider when PoB shows one
local function affixTooltips()
	local drop = tab().controls.displayItemAffix1
	if not (tab().displayItem and drop:IsShown()) then return nil end
	local last = #drop.list - 1
	local out = {}
	for _, index in ipairs({ 0, math.floor(last / 2), last }) do
		local dup = false
		for _, seen in ipairs(out) do dup = dup or seen.index == index end
		if not dup then out[#out + 1] = { slot = 0, index = index, tooltip = affixTooltip(0, index) } end
	end
	if drop.slider:IsShown() then out[#out + 1] = { slot = 0, tooltip = affixTooltip(0) } end
	return out
end

-- The text the tree's layout key is made of, read from the nodes the spec holds: the tree
-- version, then each placed node by id with its name, stat lines, type, art and ascendancy
function layoutDigest()
	local nodes = {}
	for _, node in pairs(build.spec.nodes) do
		if node.x then nodes[#nodes + 1] = node end
	end
	table.sort(nodes, function(a, b) return a.id < b.id end)
	local lines = { build.spec.treeVersion }
	for _, node in ipairs(nodes) do
		lines[#lines + 1] = table.concat({
			tostring(node.id), node.dn or "", table.concat(node.sd or {}, "\1"), node.type or "",
			node.icon or "", node.inactiveIcon or "", node.activeIcon or "", node.ascendancyName or "",
		}, "\2")
	end
	return table.concat(lines, "\n")
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
	local before = #actions
	treeAlloc(node)
	assert(#actions == before + 1 and actions[#actions].alloc == node.id, "tree_lib recorded something else")
	actions[#actions] = { cmd = "AllocNode", id = node.id }
end

function act.dealloc(node)
	local before = #actions
	treeDealloc(node)
	assert(#actions == before + 1 and actions[#actions].dealloc == node.id, "tree_lib recorded something else")
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

-- The Skills tab's New button (SkillListControl.lua:38-52), for the weapon-set recipe
function act.add_group()
	build.skillsTab.controls.groupList.controls.new.onClick()
	record({ cmd = "AddSocketGroup" })
end

-- A group's slot dropdown (SkillsTab.lua:168-184), after a click on its row
function act.group_slot(index, slotName)
	local tabS = build.skillsTab
	local group = tabS.socketGroupList[index + 1]
	assert(group, "no socket group " .. index)
	local list = tabS.controls.groupList
	list.selIndex, list.selValue = index + 1, group
	list:OnSelect(index + 1, group)
	local drop = tabS.controls.groupSlot
	local found
	for i, choice in ipairs(drop.list) do
		if choice.slotName == slotName then
			drop:SetSel(i)
			found = true
		end
	end
	assert(found, "no slot " .. tostring(slotName))
	record({ cmd = "SetGroupSlot", group = index, slot = slotName })
end

-- ---- Item sets ----

-- Opens the Manage Item Sets popup (ItemsTab.lua:2506), calls `fn(list)` with its
-- ItemSetListControl and closes what is left open.
local function withSetList(fn)
	local depth = #main.popups
	tab():OpenItemSetManagePopup()
	fn(main.popups[1].controls.setList)
	while #main.popups > depth do main:ClosePopup() end
end

local function setIndex(id)
	for i, setId in ipairs(tab().itemSetOrderList) do
		if setId == id then return i end
	end
	error("no item set " .. tostring(id))
end

-- Types the title into the name popup on top and presses Save (ItemSetListControl.lua:44-70)
local function saveName(title)
	local popup = main.popups[1]
	popup.controls.edit:SetText(title, true)
	assert(popup.controls.save.enabled, "Save is disabled")
	popup.controls.save.onClick()
end

-- The New button and the name popup; returns the new set's id
function act.new_set(title)
	local before = {}
	for _, id in ipairs(tab().itemSetOrderList) do before[id] = true end
	withSetList(function(list)
		list.controls.new.onClick()
		saveName(title)
	end)
	local made
	for _, id in ipairs(tab().itemSetOrderList) do
		if not before[id] then made = id end
	end
	record({ cmd = "NewItemSet", title = title })
	return made
end

-- The Copy button and the name popup; returns the copy's id
function act.copy_set(id, title)
	local before = {}
	for _, listed in ipairs(tab().itemSetOrderList) do before[listed] = true end
	withSetList(function(list)
		list.selIndex, list.selValue = setIndex(id), id
		list.controls.copy.onClick()
		saveName(title)
	end)
	local made
	for _, listed in ipairs(tab().itemSetOrderList) do
		if not before[listed] then made = listed end
	end
	record({ cmd = "CopyItemSet", id = id, title = title })
	return made
end

-- The Rename button and the name popup
function act.rename_set(id, title)
	setIndex(id)
	withSetList(function(list)
		list.selIndex, list.selValue = setIndex(id), id
		list.controls.rename.onClick()
		saveName(title)
	end)
	record({ cmd = "RenameItemSet", id = id, title = title })
end

-- The Delete button and its confirmation
function act.delete_set(id)
	withSetList(function(list)
		list.selIndex, list.selValue = setIndex(id), id
		list.controls.delete.onClick()
		confirm()
	end)
	record({ cmd = "DeleteItemSet", id = id })
end

-- The set dropdown (ItemsTab.lua:99-102), after Draw has put the titles in it
function act.select_set(id)
	local t = tab()
	local select = t.controls.setSelect
	local titles = {}
	for i, setId in ipairs(t.itemSetOrderList) do
		titles[i] = t.itemSets[setId].title or "Default"
		if setId == t.activeItemSetId then select.selIndex = i end
	end
	select:SetList(titles)
	select:SetSel(setIndex(id))
	record({ cmd = "SelectItemSet", id = id })
end

-- Dragging the set to a new place in the manager's list and dropping it; `to` is the final
-- position, which a drop expresses as the row it lands before (ListControl.lua:426-436).
function act.move_set(from, to)
	-- a drop on the row's own place does nothing (ListControl.lua:429)
	if from == to then
		record({ cmd = "MoveItemSet", from = from, to = to })
		return
	end
	withSetList(function(list)
		list.selIndex, list.selValue = from + 1, list.list[from + 1]
		local dragIndex = to > from and to + 2 or to + 1
		table.remove(list.list, list.selIndex)
		if dragIndex > list.selIndex then
			dragIndex = dragIndex - 1
		end
		table.insert(list.list, dragIndex, list.selValue)
		list:OnOrderChange(list.selIndex, dragIndex)
		list.selValue = nil
	end)
	record({ cmd = "MoveItemSet", from = from, to = to })
end

-- ---- The display item ----

-- Ctrl+V's body (ItemsTab.lua:1454-1458); the text is the clipboard
function act.paste(text)
	tab():CreateDisplayItemFromRaw(text, true)
	record({ cmd = "PasteItem", text = text })
end

-- The list's double click (ItemListControl.lua:315-321)
function act.edit(id)
	local list = itemList()
	local found
	for i, listed in ipairs(list.list) do
		if listed == id then found = i end
	end
	assert(found, "item " .. tostring(id) .. " is not listed")
	list:OnSelClick(found, id, true)
	record({ cmd = "EditItem", id = id })
end

-- The "Edit..." popup: the rarity dropdown, the text box, Save (ItemsTab.lua:2606-2659)
function act.edit_text(rarity, text)
	local depth = #main.popups
	tab():EditDisplayItemText()
	local popup = main.popups[1]
	popup.controls.rarity:SetSel(rarity + 1)
	popup.controls.edit:SetText(text)
	assert(popup.controls.save:IsEnabled(), "Save is disabled")
	popup.controls.save.onClick()
	while #main.popups > depth do main:ClosePopup() end
	record({ cmd = "SetDisplayItemText", rarity = rarity, text = text })
end

-- The panel's Save / Add to build button
function act.save()
	tab():UpdateSockets()
	tab().controls.addDisplayItem.onClick()
	record({ cmd = "SaveDisplayItem" })
end

-- The panel's Cancel button
function act.cancel()
	tab().controls.removeDisplayItem.onClick()
	record({ cmd = "CancelDisplayItem" })
end

-- Ctrl+D's body (ItemsTab.lua:1481-1483); nothing happens when the toggle is already there
function act.stat_differences(on)
	if (tab().showStatDifferences and true or false) ~= on then
		tab().showStatDifferences = on
		build.buildFlag = true
	end
	record({ cmd = "SetShowStatDifferences", on = on })
end

-- ---- The display item's editors ----
-- Each presses its own control. The controls' state that PoB syncs only in Draw is synced the way
-- Draw does it: by asking `IsShown()`, or `UpdateSocketControls()` for the sockets.

local function control(name)
	local c = tab().controls[name]
	assert(c, "no control " .. name)
	return c
end

-- A shown, enabled dropdown; `index` is 0-based
local function pickDrop(c, index)
	assert(c:IsShown() and c:IsEnabled(), "the dropdown is not usable")
	c:SetSel(index + 1)
end

local variantControls = { "displayItemVariant", "displayItemAltVariant", "displayItemAltVariant2",
	"displayItemAltVariant3", "displayItemAltVariant4", "displayItemAltVariant5" }

function act.variant(index, entry)
	pickDrop(control(variantControls[index + 1]), entry)
	record({ cmd = "SetItemVariant", control = index, index = entry })
end

function act.version(index)
	pickDrop(control("displayItemVersion"), index)
	record({ cmd = "SetItemVersion", index = index })
end

-- Typing into a box: select what is there, then the characters arrive one OnChar each
local function typeInto(c, text)
	assert(c:IsShown(), "the box is not shown")
	c:SelectAll()
	c:OnChar(text)
end

function act.quality(text)
	typeInto(control("displayItemQualityEdit"), text)
	record({ cmd = "SetItemQuality", text = text })
end

function act.influence(which, index)
	pickDrop(control(which == 0 and "displayItemInfluence" or "displayItemInfluence2"), index)
	record({ cmd = "SetItemInfluence", which = which, index = index })
end

function act.catalyst(index)
	pickDrop(control("displayItemCatalyst"), index)
	record({ cmd = "SetItemCatalyst", index = index })
end

function act.catalyst_quality(text)
	typeInto(control("displayItemCatalystQualityEdit"), text)
	record({ cmd = "SetItemCatalystQuality", text = text })
end

function act.socket(index, colour)
	local t = tab()
	t:UpdateSocketControls()
	local c = control("displayItemSocket" .. (index + 1))
	assert(c:IsShown(), "the socket is not shown")
	for i, entry in ipairs(c.list) do
		if entry.color == colour then c:SetSel(i) end
	end
	record({ cmd = "SetItemSocket", index = index, colour = colour })
end

-- The check box after socket `index`
function act.link(index, linked)
	local t = tab()
	t:UpdateSocketControls()
	local c = control("displayItemLink" .. (index + 1))
	assert(c:IsShown(), "the link is not shown")
	if (c.state and true or false) ~= linked then
		c.state = linked
		c.changeFunc(linked)
	end
	record({ cmd = "SetItemSocketLink", index = index, linked = linked })
end

function act.add_socket()
	local c = control("displayItemAddSocket")
	assert(c:IsShown(), "the button is not shown")
	c.onClick()
	record({ cmd = "AddItemSocket" })
end

-- The slider and the Foulborn box of range line `line` (0-based): the stacked ones of a unique
-- when PoB lists all its affixes (the default), else the single pair after the dropdown picks the
-- line
local function rangeControls(line)
	local item = tab().displayItem
	if main.showAllItemAffixes and item.rarity == "UNIQUE" then
		return control("displayItemStackedRangeSlider" .. (line + 1)),
			control("displayItemStackedMutatedCheckbox" .. (line + 1))
	end
	local drop = control("displayItemRangeLine")
	assert(drop:IsShown(), "the range line dropdown is not shown")
	drop:SetSel(line + 1)
	return control("displayItemRangeSlider"), control("displayItemMutatedCheckbox")
end

-- The Draw that precedes a click asks the slider whether it is shown, which syncs its value to
-- the line (ItemsTab.lua:1076-1088)
function act.range(line, value)
	local slider = rangeControls(line)
	assert(slider:IsShown(), "the slider is not shown")
	slider:SetVal(value)
	record({ cmd = "SetRangeLine", line = line, value = value })
end

-- The Foulborn box beside the slider
function act.mutated(line, on)
	local _, box = rangeControls(line)
	assert(box:IsShown(), "the Foulborn box is not shown")
	if (box.state and true or false) ~= on then
		box.state = on
		box.changeFunc(on)
	end
	record({ cmd = "SetRangeLineMutated", line = line, on = on })
end

-- The click on a tooltip line (ItemsTab.lua:1535); the click's body is the model method
function act.toggle_mod(list, index)
	local t = tab()
	local item = t.displayItem
	local field = ({ enchant = "enchantModLines", scourge = "scourgeModLines", implicit = "implicitModLines",
		explicit = "explicitModLines", crucible = "crucibleModLines" })[list]
	local modLine = item[field][index + 1]
	t:UpdateDisplayItemTooltip()
	local onTooltip = false
	for _, line in ipairs(t.displayItemTooltip.lines) do
		if line.modLine == modLine then onTooltip = true end
	end
	assert(onTooltip, "that mod line is not on the tooltip")
	t:ToggleDisplayItemModLine(modLine)
	record({ cmd = "ToggleModLine", line = { list = list, index = index } })
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

-- ---- The crafter ----

-- the label of a dropdown entry
local function entryLabel(entry)
	return type(entry) == "table" and entry.label or entry
end

-- Picks an entry of an affix dropdown (the gate's `SetAffix`; `index` is 0-based)
function act.affix(slot, index)
	local drop = control("displayItemAffix" .. (slot + 1))
	assert(drop:IsShown() and drop:IsEnabled(), "the affix dropdown is not usable")
	local entry = drop.list[index + 1]
	assert(entry ~= nil, "no such entry")
	drop:SetSel(index + 1)
	record({ cmd = "SetAffix", slot = slot, index = index, label = entryLabel(entry) })
end

-- The slider under an affix dropdown
function act.affix_range(slot, value)
	local slider = control("displayItemAffix" .. (slot + 1)).slider
	assert(slider:IsShown(), "the affix slider is not shown")
	slider:SetVal(value)
	record({ cmd = "SetAffixRange", slot = slot, value = value })
end

-- "Modifier sorting"
function act.crafting_sort(index)
	pickDrop(control("craftingSorting"), index)
	record({ cmd = "SetCraftingSort", index = index })
end

function act.cluster_skill(index)
	pickDrop(control("displayItemClusterJewelSkill"), index)
	record({ cmd = "SetClusterSkill", index = index })
end

-- The "Added Passives" slider, which takes 0..1 across the jewel's node counts
function act.cluster_count(count)
	local jewel = tab().displayItem.clusterJewel
	local slider = control("displayItemClusterJewelNodeCount")
	assert(slider:IsShown(), "the node count slider is not shown")
	slider:SetVal((count - jewel.minNodes) / (jewel.maxNodes - jewel.minNodes))
	record({ cmd = "SetClusterNodeCount", count = count })
end

-- The Items tab right now, recorded for the gate to compare with ours at this point. `layout`
-- adds the tree layout's text.
function checkpoint(layout)
	frame()
	local jewels = {}
	for node, id in pairs(build.spec.jewels) do
		if id ~= 0 then jewels[node] = id end
	end
	table.insert(actions, { check = { items = itemsDump(), jewels = jewels, sidebar = sidebar_lines(), display = displayDump(), main_group = build.mainSocketGroup, popup = popupDump and popupDump() or nil,
		affix_tooltips = affixTooltips(), layout = layout and layoutDigest() or nil } })
end
