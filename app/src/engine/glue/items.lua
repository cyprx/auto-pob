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

-- ---- Item tooltips (R1, R3) ----

-- A PoB `Tooltip` as data. PoB's own `AddSeparator` already dropped back-to-back separators, so
-- every line of `tt.lines` is sent as it is; a line without text is a separator.
-- `mod_line` is only set for the display item's tooltip (never here).
local function read_tooltip(tt)
	local out = { influences = {}, lines = {} }
	if tt.tooltipHeader then
		out.header = tostring(tt.tooltipHeader)
	end
	for _, name in ipairs({ "influenceHeader1", "influenceHeader2" }) do
		if tt[name] then
			table.insert(out.influences, tostring(tt[name]))
		end
	end
	if tt.foilType then
		out.foil = tostring(tt.foilType)
	end
	-- a string colour code; PoB's default is an {r, g, b} table, which only an empty tooltip has
	if type(tt.color) == "string" then
		out.colour = tt.color
	end
	for i, line in ipairs(tt.lines) do
		if line.text then
			out.lines[i] = {
				size = line.size,
				text = line.text,
				font = line.font,
				center = line.center and true or false,
				background = type(line.background) == "string" and line.background or nil,
			}
		else
			out.lines[i] = { separator = line.size }
		end
	end
	return out
end

local function index_of(list, id)
	for i, value in ipairs(list) do
		if value == id then
			return i
		end
	end
end

-- What hovering an item shows: in the item list when `slot_name` is nil, else over the slot or
-- one of its dropdown entries. Nil when the item, the slot or the entry is gone.
function glue.item_tooltip(id, slot_name)
	local tab = items_tab()
	local list = tab.controls.itemList
	-- the updates the tab's Draw runs before a hover can happen (R16)
	list:UpdateList()
	tab:UpdateSockets()
	if not tab.items[id] then
		return nil
	end
	local tooltip = new("Tooltip")
	if slot_name == nil then
		local index = index_of(list.list, id)
		if not index then
			return nil
		end
		list:AddValueTooltip(tooltip, index, id)
	else
		local slot = tab.slots[slot_name]
		local index = slot and index_of(slot.items, id)
		if not index then
			return nil
		end
		slot.tooltipFunc(tooltip, "HOVER", index, id)
	end
	return read_tooltip(tooltip)
end

-- The Copy button: ItemListControl:OnSelCopy (ItemListControl.lua:326). The global `Copy` is
-- swapped for a capture during the call and always put back (R14).
function glue.item_copy(id)
	local tab = items_tab()
	if not tab.items[id] then
		error("no such item", 0)
	end
	local list = tab.controls.itemList
	local saved, text = Copy, nil
	Copy = function(copied)
		text = copied
	end
	local ok, err = pcall(list.OnSelCopy, list, index_of(tab.itemOrderList, id), id)
	Copy = saved
	if not ok then
		error(err, 0)
	end
	return text or ""
end

-- ---- The item list and the slots (R5, R6, R8, R12) ----

-- The list as the tab shows it: the draw-only update first (R16)
local function synced_list()
	local list = items_tab().controls.itemList
	list:UpdateList()
	items_tab():UpdateSockets()
	return list
end

-- The 1-based position of a value in a list
local function position(list, value)
	for i, v in ipairs(list) do
		if v == value then
			return i
		end
	end
end

-- The Sort button
function glue.items_sort()
	synced_list().controls.sort.onClick()
	glue.frame()
end

-- The Delete button, which runs `OnSelDelete` on the selected row. A row the filter hides
-- can't be selected (R8). PoB asks first when the item is in use.
function glue.items_delete(id)
	local tab = items_tab()
	local list = synced_list()
	local index = position(list.list, id)
	if not (index and tab.items[id]) then
		error("no such item", 0)
	end
	local popups = pob.main.main.popups
	glue.in_popups(function()
		local depth = #popups
		list:OnSelDelete(index, id)
		if #popups > depth then
			glue.confirm_popup("delete the item")
		end
	end)
end

-- The Del Unused button, which has no confirmation; it is disabled with an empty list
function glue.items_delete_unused()
	local list = synced_list()
	if #list.list > 0 then
		list.controls.deleteUnused.onClick()
	end
end

-- The Del All button and its confirmation; disabled with an empty list
function glue.items_delete_all()
	local list = synced_list()
	if #list.list == 0 then
		return
	end
	glue.in_popups(function()
		list.controls.deleteAll.onClick()
		glue.confirm_popup("delete the items")
	end)
end

-- ListControl's drop (ListControl.lua:426-436); `to` is the item's final position. Only the
-- unfiltered list is mutable.
function glue.items_move(from, to)
	local tab = items_tab()
	local list = synced_list()
	if not list.isMutable then
		error("the list can't be reordered while it is filtered", 0)
	end
	local id = tab.itemOrderList[from]
	if not (id and tab.itemOrderList[to]) then
		error("no such item", 0)
	end
	if from == to then
		return
	end
	list.selIndex, list.selValue = from, id
	table.remove(list.list, from)
	table.insert(list.list, to, id)
	list:OnOrderChange(from, to)
	list.selValue = nil
	list.selIndex = nil
	glue.frame()
end

-- The loadout filter dropdown
function glue.items_set_filter(index)
	local list = synced_list()
	local filter = list.controls.loadoutFilter
	if index < 1 or index > #filter.list then
		error("no such filter", 0)
	end
	filter:SetSel(index)
end

-- The slot dropdown's `selFunc` (ItemSlotControl.lua:12-19); `id` nil is "None" (R5)
function glue.items_equip(slot_name, id)
	local tab = items_tab()
	synced_list()
	local slot = tab.slots[slot_name]
	if not slot or not slot:IsShown() then
		error("no such slot", 0)
	end
	local index = 1
	if id ~= nil then
		if not tab.items[id] then
			error("no such item", 0)
		end
		index = position(slot.items, id)
		if not index or id == 0 then
			error("that item can't go in that slot", 0)
		end
	end
	slot.selFunc(index, slot.list[index])
end

-- A flask's own checkbox
function glue.items_set_flask(slot_name, on)
	local tab = items_tab()
	synced_list()
	local slot = tab.slots[slot_name]
	if not (slot and slot:IsShown()) then
		error("no such slot", 0)
	end
	local activate = slot.controls.activate
	if not activate then
		error("not a flask slot", 0)
	end
	if not activate:IsEnabled() then
		error("that flask slot is empty", 0)
	end
	if on ~= (slot.active and true or false) then
		glue.press_check(activate, on)
	end
end

-- The "I" and "II" buttons; the one for the active set is locked (R6)
function glue.items_select_weapon_set(second)
	local tab = items_tab()
	local button = second and tab.controls.weaponSwap2 or tab.controls.weaponSwap1
	if not button.locked() then
		button.onClick()
	end
end

-- Ctrl+Z and Ctrl+Y on the tab (ItemsTab.lua:1458-1465); the display item isn't covered (R12)
function glue.items_undo()
	items_tab():Undo()
	pob.build.buildFlag = true
end

function glue.items_redo()
	items_tab():Redo()
	pob.build.buildFlag = true
end
