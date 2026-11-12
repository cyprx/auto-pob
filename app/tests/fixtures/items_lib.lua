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
	-- the "Passive tree:" dropdown, its list set as the tab's Draw sets it
	local specSelect = t.controls.specSelect
	specSelect:SetList(build.treeTab:GetSpecList())
	out.specs = { labels = {}, selected = specSelect.selIndex - 1, enabled = specSelect:IsEnabled() and true or false }
	for i, label in ipairs(specSelect.list) do out.specs.labels[i] = label end
	-- the shared lists (NF-2): the rows as PoB's list draws them; the set rows and tooltips
	-- through a set list control of the kind the Manage Item Sets window shows
	out.shared_items, out.shared_sets = {}, {}
	out.shared_shown = t.controls.sharedItemList:IsShown() and true or false
	for i, item in ipairs(main.sharedItemList) do
		out.shared_items[i] = { name = item.name, rarity = item.rarity, label = t.controls.sharedItemList:GetRowValue(1, i, item) }
	end
	local setList = new("SharedItemSetListControl", nil, { 0, 0, 300, 200 }, t)
	for i, set in ipairs(main.sharedItemSetList) do
		local tt = new("Tooltip")
		setList:AddValueTooltip(tt, i, set)
		local lines = {}
		for _, line in ipairs(tt.lines) do
			if line.text then lines[#lines + 1] = line.text end
		end
		out.shared_sets[i] = { title = setList:GetRowValue(1, i, set), lines = lines, raw_title = set.title }
	end
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
	-- the child tooltips PoB draws beside it (the granted skills under Shift), the empty ones
	-- drawing nothing
	local children = {}
	for _, child in ipairs(tt.childTooltips or {}) do
		if #child.lines > 0 then children[#children + 1] = tooltipDump(child) end
	end
	if #children > 0 then out.children = children end
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
	-- the crafted, crucible and custom lines, from the model: the Remove rows' labels are all
	-- "..." headless
	out.custom_mods = {}
	local rare = item.rarity == "MAGIC" or item.rarity == "RARE" or item.rareLikeUnique
	for _, list in ipairs({ item.explicitModLines, item.crucibleModLines or {} }) do
		for _, modLine in ipairs(list) do
			local line = itemLib.formatModLine(modLine)
			if line and (rare or #(item.crucibleModLines or {}) > 0)
				and (modLine.custom or modLine.crafted or modLine.crucible) then
				local kind = "Custom"
				if modLine.crafted then kind = "Crafted" elseif modLine.crucible then kind = "Crucible" end
				table.insert(out.custom_mods, { kind = kind, line = line })
			end
		end
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
	-- one slider and box per line instead of the line dropdown (showAllItemAffixes)
	if item.rangeLineList[1] and not c.displayItemRangeLine:IsShown() then out.stacked = true end
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

-- A custom mod's Remove button (`index` is 0-based in displayDump().custom_mods). PoB's Draw
-- refreshes the rows first.
function act.remove_custom(index)
	tab():UpdateCustomControls()
	local button = control("displayItemCustomModifierRemove" .. (index + 1))
	assert(button:IsShown(), "the Remove row is not shown")
	button.onClick()
	record({ cmd = "RemoveCustomMod", index = index })
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

-- ---- The item databases ----
-- Written against PoB's own ItemDBControl: its dropdowns are set with SetSel and its search box
-- with SetText, then the control's real Draw runs until its list builder has ended. Nothing
-- here may call our glue.

local dbFields = { "slot", "kind", "league", "requirement", "obtainable", "sort", "search_mode" }
local dbControlNames = { slot = "slot", kind = "type", league = "league", requirement = "requirement",
	obtainable = "obtainable", sort = "sort", search_mode = "searchMode" }
local viewPort = { x = 0, y = 0, width = 1920, height = 1080 }

local function dbControl(db)
	local c = tab().controls[db == "Uniques" and "uniqueDB" or "rareDB"]
	assert(c, "no database " .. tostring(db))
	return c
end

-- PoB's first frame loads both databases here: GetTime is 0, so its coroutine never yields
local function dbLoaded()
	for _ = 1, 50 do
		if not main.uniqueDB.loading and not main.rareDB.loading then return end
		frame()
	end
	error("the databases did not load")
end

local function plain(text)
	return (tostring(text or ""):gsub("%^x%x%x%x%x%x%x", ""):gsub("%^%d", ""))
end

-- Draw's own step once a database has loaded: the leagues and types the filters list
local function dbPrepared(db)
	dbLoaded()
	local c = dbControl(db)
	if not c.leaguesAndTypesLoaded then c:LoadLeaguesAndTypes() end
	return c
end

-- The 0-based index of the entry of a database dropdown with this label (colour codes removed)
function dbIndex(db, field, label)
	local drop = dbPrepared(db).controls[dbControlNames[field]]
	for i, entry in ipairs(drop.list) do
		if plain(type(entry) == "table" and entry.label or entry) == label then return i - 1 end
	end
	error("no " .. field .. " entry " .. label)
end

-- Sets every control of the panel: a field the query leaves out is reset
local function dbSet(db, query)
	local c = dbPrepared(db)
	for _, field in ipairs(dbFields) do
		local drop = c.controls[dbControlNames[field]]
		if drop then
			local want = query[field] or 0
			if type(want) == "string" then want = dbIndex(db, field, want) end
			assert(want >= 0 and want < #drop.list, field .. " is out of range")
			drop:SetSel(want + 1)
		else
			assert((query[field] or 0) == 0, db .. " has no " .. field)
		end
	end
	c.controls.search:SetText(query.search or "", true)
end

-- Draws the list control until PoB's list builder has ended. A stat sort finishes in one Draw
-- here: the builder yields only after 50 ms of GetTime, which is 0.
local function dbDrain(db)
	local c = dbControl(db)
	-- the control's height reads the tab's `maxY`, which the tab's own Draw sets (ItemsTab.lua:1447)
	-- to the bottom of its view port; the tab is never drawn here
	tab().maxY = tab().maxY or viewPort.y + viewPort.height
	local draws = 0
	repeat
		c:Draw(viewPort)
		draws = draws + 1
		assert(draws < 100000, "the list builder did not end")
	until c.listBuilder == nil
	return c, draws
end

-- The panel as plain tables, in the shape of `ItemDbView`, after the query's list is built
function dbDump(db, query)
	if query then dbSet(db, query) end
	local c, draws = dbDrain(db)
	local function drop(d)
		local labels = {}
		for i, entry in ipairs(d.list) do labels[i] = type(entry) == "table" and entry.label or entry end
		return { labels = labels, selected = d.selIndex - 1, enabled = d:IsEnabled() and true or false }
	end
	local stat = c.sortDetail ~= nil and c.sortDetail.stat ~= nil
	local rows = {}
	for i, item in ipairs(c.list) do
		local power = stat and item.measuredPower or nil
		if power and (power == math.huge or power == -math.huge or power ~= power) then power = nil end
		rows[i] = { name = item.name, label = c:GetRowValue(1, i, item), base = item.baseName or "", sort_value = power }
	end
	local k = c.controls
	return {
		db = db, loading = false, sorting = false,
		slot = drop(k.slot), kind = drop(k.type),
		league = k.league and drop(k.league) or nil, requirement = k.requirement and drop(k.requirement) or nil,
		obtainable = k.obtainable and drop(k.obtainable) or nil, sort = k.sort and drop(k.sort) or nil,
		search = k.search.buf, search_mode = drop(k.searchMode),
		rows = rows, note = plain(c.defaultText), draws = draws,
	}
end

-- Hovering a row: the control's own tooltip hook on a fresh Tooltip
function dbTooltip(db, name)
	dbLoaded()
	local c = dbControl(db)
	local item = c.db.list[name]
	if not item then return nil end
	local tt = new("Tooltip")
	c:AddValueTooltip(tt, 1, item)
	return tooltipDump(tt)
end

-- Sets the panel's filters, as typing into it does, and records the command
function act.db_open(db, query)
	dbSet(db, query)
	local cmd = { cmd = "SetItemDb", db = db, search = query.search or "" }
	for _, field in ipairs(dbFields) do
		local want = query[field] or 0
		if type(want) == "string" then want = dbIndex(db, field, want) end
		cmd[field] = want
	end
	record(cmd)
	dbDrain(db)
end

-- The row's position in the shown list, or 1
local function dbRow(db, name)
	dbDrain(db)
	local c = dbControl(db)
	local item = c.db.list[name]
	assert(item, "no database item " .. name)
	for i, listed in ipairs(c.list) do
		if listed == item then return c, item, i end
	end
	return c, item, 1
end

-- The double click (ItemDBControl.lua:359-366)
function act.db_view(db, name)
	local c, item, index = dbRow(db, name)
	c:OnSelClick(index, item, true)
	record({ cmd = "ViewDbItem", db = db, name = name })
end

-- Ctrl+click (Ctrl+Shift+click with `second`): the key state is swapped for the call, as the
-- Ctrl+V and Copy hooks are
function act.db_add(db, name, second)
	local c, item, index = dbRow(db, name)
	local saved = IsKeyDown
	IsKeyDown = function(key) return key == "CTRL" or (second and key == "SHIFT") end
	local ok, err = pcall(c.OnSelClick, c, index, item, false)
	IsKeyDown = saved
	assert(ok, err)
	record({ cmd = "AddDbItem", db = db, name = name, second_slot = second or false })
end

-- A row dragged onto a slot and released there: the row is picked up, the drag starts as the
-- list's Draw starts it (ListControl.lua:146-156), the slot under the cursor becomes the target
-- (ListControl.lua:166-176) and the button's release is PoB's own (ListControl.lua:423-452)
function act.db_drop(db, name, slotName)
	local c, item, index = dbRow(db, name)
	local slot = tab().slots[slotName]
	tab().controls.itemList:UpdateList()
	tab():UpdateSockets()
	c:SelectIndex(index)
	c.selDragging = true
	c.selDragActive = true
	c.dragType, c.dragValue = c:GetDragValue(c.selIndex, c.selValue)
	for _, target in ipairs(c.dragTargetList) do
		if not target.CanReceiveDrag or target:CanReceiveDrag(c.dragType, c.dragValue) then
			target.otherDragSource = c
		end
	end
	assert(slot:IsShown() and slot.otherDragSource == c, slotName .. " doesn't take " .. name)
	c.dragTarget = slot
	c:OnKeyUp("LEFTBUTTON")
	record({ cmd = "DropDbItem", db = db, name = name, slot = slotName })
end

-- Ctrl+click on an item list row (Ctrl+Shift+click with `second`), keys swapped for the call
function act.quick_equip(id, second)
	local list = itemList()
	local index
	for i, listed in ipairs(list.list) do
		if listed == id then index = i end
	end
	assert(index, "item " .. id .. " is not listed")
	local saved = IsKeyDown
	IsKeyDown = function(key) return key == "CTRL" or (second and key == "SHIFT") end
	local ok, err = pcall(list.OnSelClick, list, index, id, false)
	IsKeyDown = saved
	assert(ok, err)
	record({ cmd = "QuickEquipItem", id = id, second_slot = second or false })
end

-- ---- Shared items (NF-2) ----

-- A row of `source` dragged onto `target` and released there, as act.db_drop does: `at` is the
-- row the target's Draw marks under the cursor
local function sharedDrop(source, index, target, at)
	-- each Draw starts with no drop row of its own (ListControl.lua:157)
	source.selDragIndex = nil
	source:SelectIndex(index)
	source.selDragging = true
	source.selDragActive = true
	source.dragType, source.dragValue = source:GetDragValue(source.selIndex, source.selValue)
	for _, t in ipairs(source.dragTargetList) do
		if not t.CanReceiveDrag or t:CanReceiveDrag(source.dragType, source.dragValue) then
			t.otherDragSource = source
		end
	end
	assert(target.otherDragSource == source, "the target doesn't take it")
	-- Draw marks a drop row only on a list that can be reordered (ListControl.lua:157-164)
	target.selDragIndex = target.isMutable and at or nil
	source.dragTarget = target
	source:OnKeyUp("LEFTBUTTON")
	target.selDragIndex = nil
	assert(source.selValue == nil, "the drop didn't happen")
	frame()
end

local function sharedList()
	return tab().controls.sharedItemList
end

-- a build item dragged onto the shared list at 0-based row `at`
function act.shared_add_item(id, at)
	local list = itemList()
	for i, listed in ipairs(list.list) do
		if listed == id then sharedDrop(list, i, sharedList(), at + 1) return end
	end
	error("item " .. id .. " is not listed")
end

function act.shared_add_db(db, name, at)
	local c, _, index = dbRow(db, name)
	sharedDrop(c, index, sharedList(), at + 1)
end

-- shared item `index` (0-based) dragged onto a slot, or onto the item list at 0-based row `at`
function act.shared_drop(index, slotName, at)
	tab():UpdateSockets()
	itemList():UpdateList()
	local target = slotName and tab().slots[slotName] or itemList()
	sharedDrop(sharedList(), index + 1, target, at and at + 1)
end

-- a database row dragged onto the item list at 0-based row `at` (ItemsTab.lua:1184-1187)
function act.db_drop_on_list(db, name, at)
	local c, _, index = dbRow(db, name)
	itemList():UpdateList()
	sharedDrop(c, index, itemList(), at and at + 1)
end

-- Ctrl+C on shared item `index` (0-based), PoB's ListControl key path with `Copy` caught
function act.shared_copy(index)
	local list = sharedList()
	list:SelectIndex(index + 1)
	local saved, text = Copy, nil
	Copy = function(copied) text = copied end
	local savedKey = IsKeyDown
	IsKeyDown = function(key) return key == "CTRL" end
	local ok, err = pcall(list.OnKeyDown, list, "c")
	IsKeyDown, Copy = savedKey, saved
	assert(ok, err)
	return text
end

-- an item dragged onto the sidebar's minion dropdown: a build item (`kind` "item", `a` its id),
-- a database row ("db", `a` the database, `b` the name) or a shared item ("shared", 0-based `a`)
function act.minion_drop(kind, a, b)
	build:RefreshSkillSelectControls(build.controls, build.mainSocketGroup, "")
	local target = build.controls.mainSkillMinion
	if kind == "item" then
		local list = itemList()
		list:UpdateList()
		for i, listed in ipairs(list.list) do
			if listed == a then sharedDrop(list, i, target, nil) return end
		end
		error("item " .. a .. " is not listed")
	elseif kind == "db" then
		local c, _, index = dbRow(a, b)
		sharedDrop(c, index, target, nil)
	else
		sharedDrop(sharedList(), a + 1, target, nil)
	end
end

-- the Delete button and its confirmation
function act.shared_delete(index)
	local list = sharedList()
	list:SelectIndex(index + 1)
	list.controls.delete.onClick()
	confirm()
end

-- a drag within the list: `to` is the final 0-based position
function act.shared_move(from, to)
	local list = sharedList()
	list:SelectIndex(from + 1)
	list.selDragging = true
	list.selDragActive = true
	list.selDragIndex = to > from and to + 2 or to + 1
	list:OnKeyUp("LEFTBUTTON")
end

function act.shared_view(index)
	local list = sharedList()
	list:OnSelClick(index + 1, main.sharedItemList[index + 1], true)
end

-- The Items tab's "Manage..." (ItemsTab.lua:113-116): PoB's Manage Item Sets window
local function withSetManager(fn)
	local depth = #main.popups
	tab().controls.setManage.onClick()
	assert(#main.popups == depth + 1, "Manage... opened no window")
	local c = main.popups[1].controls
	fn(c.setList, c.sharedList)
	while #main.popups > depth do main:ClosePopup() end
end

function act.shared_set_add(id)
	withSetManager(function(setList, shared)
		for i, value in ipairs(setList.list) do
			if value == id then sharedDrop(setList, i, shared, nil) return end
		end
		error("no set " .. id)
	end)
end

function act.shared_set_import(index)
	withSetManager(function(setList, shared)
		sharedDrop(shared, index + 1, setList, nil)
	end)
end

function act.shared_set_delete(index)
	withSetManager(function(_, shared)
		shared:SelectIndex(index + 1)
		shared.controls.delete.onClick()
		confirm()
	end)
end

function act.shared_set_rename(index, title)
	withSetManager(function(_, shared)
		shared:SelectIndex(index + 1)
		shared.controls.rename.onClick()
		local popup = main.popups[1]
		popup.controls.edit:SetText(title, true)
		popup.controls.save.onClick()
	end)
end

function act.shared_set_move(from, to)
	withSetManager(function(_, shared)
		shared:SelectIndex(from + 1)
		shared.selDragging = true
		shared.selDragActive = true
		shared.selDragIndex = to > from and to + 2 or to + 1
		shared:OnKeyUp("LEFTBUTTON")
	end)
end
