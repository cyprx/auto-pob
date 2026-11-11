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
-- With the displayed `item`, a line that toggles a mod (PoB's `modLine`, found by identity in the
-- item's lists) carries `mod_line` (R4); the item list's tooltips never pass one.
local mod_line_lists = {
	{ "enchant", "enchantModLines" },
	{ "scourge", "scourgeModLines" },
	{ "implicit", "implicitModLines" },
	{ "explicit", "explicitModLines" },
	{ "crucible", "crucibleModLines" },
}

local function mod_line_ref(item, mod_line)
	for _, entry in ipairs(mod_line_lists) do
		for i, candidate in ipairs(item[entry[2]]) do
			if candidate == mod_line then
				return { list = entry[1], index = i - 1 }
			end
		end
	end
end

local function read_tooltip(tt, item)
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
				mod_line = item and line.modLine and mod_line_ref(item, line.modLine) or nil,
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

-- ---- Item sets (R9) ----

local function set_position(id)
	local position_of = position(items_tab().itemSetOrderList, id)
	if not position_of then
		error("no such item set", 0)
	end
	return position_of
end

-- Opens the Manage Item Sets popup (ItemsTab.lua:2506), calls `fn(list)` with its
-- ItemSetListControl and closes what is left open. The shared list beside it is built too.
local function with_set_list(fn)
	glue.in_popups(function()
		items_tab():OpenItemSetManagePopup()
		local list = pob.main.main.popups[1].controls.setList
		if not list then
			error("couldn't open the item set list", 0)
		end
		return fn(list)
	end)
end

local function check_title(title)
	if not title:gsub("\r", ""):match("%S") then
		error("the item set needs a name", 0)
	end
end

-- Types the title into the name popup `RenameSet` opened and presses Save (it stays disabled
-- until the name has a visible character). When that fails, Cancel is pressed, which drops a
-- set that New or Copy had already made.
local function save_name(title)
	local popup = pob.main.main.popups[1]
	local ok, applied = pcall(glue.type_into, popup.controls.edit, title)
	if ok and applied and popup.controls.save.enabled then
		popup.controls.save.onClick()
		return
	end
	popup.controls.cancel.onClick()
	if not ok then
		error(applied, 0)
	end
	check_title("")
end

-- Draw's sync of the set dropdown (ItemsTab.lua:1508-1516), then the user's pick
function glue.items_select_set(id)
	local tab = items_tab()
	local index = set_position(id)
	local select = tab.controls.setSelect
	local titles = {}
	for i, set_id in ipairs(tab.itemSetOrderList) do
		titles[i] = tab.itemSets[set_id].title or "Default"
		if set_id == tab.activeItemSetId then
			select.selIndex = i
		end
	end
	select:SetList(titles)
	select:SetSel(index)
end

-- The New button and the name popup
function glue.items_new_set(title)
	check_title(title)
	with_set_list(function(list)
		list.controls.new.onClick()
		save_name(title)
	end)
end

-- The Rename button and the name popup
function glue.items_rename_set(id, title)
	set_position(id)
	check_title(title)
	with_set_list(function(list)
		list:RenameSet(items_tab().itemSets[id])
		save_name(title)
	end)
	glue.frame()
end

-- The Copy button and the name popup
function glue.items_copy_set(id, title)
	local index = set_position(id)
	check_title(title)
	with_set_list(function(list)
		list.selIndex, list.selValue = index, id
		list.controls.copy.onClick()
		save_name(title)
	end)
end

-- OnSelDelete and its confirmation
function glue.items_delete_set(id)
	local index = set_position(id)
	if #items_tab().itemSetOrderList <= 1 then
		error("the last item set can't be deleted", 0)
	end
	with_set_list(function(list)
		list.selIndex, list.selValue = index, id
		list:OnSelDelete(index, id)
		glue.confirm_popup("delete the item set")
	end)
end

-- ListControl's drop, then OnOrderChange (ItemSetListControl.lua:108); `to` is the final place
function glue.items_move_set(from, to)
	local tab = items_tab()
	local id = tab.itemSetOrderList[from]
	if not (id and tab.itemSetOrderList[to]) then
		error("no such item set", 0)
	end
	if from == to then
		return
	end
	with_set_list(function(list)
		list.selIndex, list.selValue = from, id
		table.remove(list.list, from)
		table.insert(list.list, to, id)
		list:OnOrderChange(from, to)
		list.selValue = nil
		list.selIndex = nil
	end)
	glue.frame()
end

-- ---- The display item (R4, R10, R16, R23) ----

-- PoB builds the display tooltip only when the item is edited, so after a recalculation or a
-- Ctrl+D its comparison is stale. The key it was last built for is kept here and dropped
-- when another item is displayed.
local tooltip_key

local function fresh_display_tooltip(tab)
	local key = { item = tab.displayItem, revision = pob.build.outputRevision, diff = tab.showStatDifferences }
	if not (tooltip_key
		and tooltip_key.item == key.item
		and tooltip_key.revision == key.revision
		and tooltip_key.diff == key.diff) then
		tab:UpdateDisplayItemTooltip()
		tooltip_key = key
	end
end

-- A dropdown as the panel shows it; `enabled` is the control's own, which can be a function
local function drop_view(control)
	local view = glue.drop_list(control)
	view.enabled = control:IsEnabled() and true or false
	return view
end

-- The readers of the panel's parts, each `function(tab, item, out)`. SP5b adds its sections
-- (affixes, cluster jewels, custom mods) to this list.
local display_sections = {}

-- the header, and what PoB's Edit popup opens with: read from the popup, which is closed again
table.insert(display_sections, function(tab, item, out)
	out.id = item.id
	out.name = item.name
	out.base = item.baseName or ""
	out.rarity = item.rarity
	out.save_label = tab.controls.addDisplayItem:GetProperty("label")
	glue.in_popups(function()
		tab:EditDisplayItemText()
		local popup = pob.main.main.popups[1]
		out.edit_rarity = popup.controls.rarity.selIndex - 1
		out.edit_text = popup.controls.edit.buf
	end)
end)

-- the version and the shown variant dropdowns
table.insert(display_sections, function(tab, item, out)
	local c = tab.controls
	if c.displayItemVersion:IsShown() then
		out.version = drop_view(c.displayItemVersion)
	end
	out.variants = {}
	local names = {
		"displayItemVariant", "displayItemAltVariant", "displayItemAltVariant2",
		"displayItemAltVariant3", "displayItemAltVariant4", "displayItemAltVariant5",
	}
	for i, name in ipairs(names) do
		if c[name]:IsShown() then
			table.insert(out.variants, { control = i - 1, list = drop_view(c[name]) })
		end
	end
end)

-- the sockets, from the model, and whether PoB shows each one's control
table.insert(display_sections, function(tab, item, out)
	out.sockets = {}
	local count = #item.sockets - item.abyssalSocketCount
	for i = 1, count do
		local control = tab.controls["displayItemSocket" .. i]
		if not control then
			break
		end
		local socket = item.sockets[i]
		local linked
		if i < count then
			linked = item.sockets[i + 1].group == socket.group
		end
		out.sockets[i] = {
			colour = socket.color,
			shown = control:IsShown() and true or false,
			linked = linked,
		}
	end
	out.can_add_socket = tab.controls.displayItemAddSocket:IsShown() and true or false
end)

-- influence, quality and catalyst: only while PoB shows the control
table.insert(display_sections, function(tab, item, out)
	local c = tab.controls
	out.influences = {}
	if c.displayItemInfluence:IsShown() then
		out.influences = { drop_view(c.displayItemInfluence), drop_view(c.displayItemInfluence2) }
	end
	if c.displayItemQualityEdit:IsShown() then
		out.quality = c.displayItemQualityEdit.buf
	end
	if c.displayItemCatalyst:IsShown() then
		out.catalyst = drop_view(c.displayItemCatalyst)
	end
	if c.displayItemCatalystQualityEdit:IsShown() then
		out.catalyst_quality = c.displayItemCatalystQualityEdit.buf
	end
end)

-- the lines with a range slider or a Foulborn box
table.insert(display_sections, function(tab, item, out)
	out.range_lines = {}
	for i, mod_line in ipairs(item.rangeLineList) do
		out.range_lines[i] = {
			label = mod_line.line,
			range = mod_line.range,
			slider = mod_line.showSlider and true or false,
			mutable = (mod_line.modId and mod_line.newModId) and true or false,
			mutated = mod_line.mutated and true or false,
		}
	end
end)

-- PoB's own tooltip for the item (R4)
table.insert(display_sections, function(tab, item, out)
	fresh_display_tooltip(tab)
	out.tooltip = read_tooltip(tab.displayItemTooltip, item)
end)

-- The display panel, or nil when no item is displayed
function glue.display_item_view()
	local tab = items_tab()
	local item = tab.displayItem
	if not item then
		tooltip_key = nil
		return nil
	end
	local view = {}
	for _, read in ipairs(display_sections) do
		read(tab, item, view)
	end
	return view
end

-- Ctrl+V's body (ItemsTab.lua:1454-1458). PoB ignores text with no item base; that is an error
-- here, and the display item stays as it was.
function glue.display_paste(text)
	local tab = items_tab()
	local before = tab.displayItem
	tab:CreateDisplayItemFromRaw(text, true)
	-- every item PoB displays is a new object
	if tab.displayItem == before then
		error("PoB couldn't read that item", 0)
	end
end

-- The list's double click (ItemListControl:OnSelClick)
function glue.display_edit(id)
	local tab = items_tab()
	local list = synced_list()
	local index = position(tab.itemOrderList, id)
	if not index then
		error("no such item", 0)
	end
	list:OnSelClick(index, id, true)
end

-- The "Edit..." / "Create custom..." popup: pick the rarity, paste the text, press Save when
-- PoB's Save is enabled (EditDisplayItemText, ItemsTab.lua:2606)
function glue.display_set_text(rarity, text)
	local tab = items_tab()
	if rarity > 4 then
		error("no such rarity", 0)
	end
	synced_list()
	glue.in_popups(function()
		tab:EditDisplayItemText()
		local popup = pob.main.main.popups[1]
		popup.controls.rarity:SetSel(rarity + 1)
		local pasted = glue.paste_into(popup.controls.edit, text)
		local save = popup.controls.save
		if not (pasted and save:IsEnabled()) then
			error("PoB couldn't read that item", 0)
		end
		save.onClick()
	end)
end

-- The panel's Save / Add to build button
function glue.display_save()
	local tab = items_tab()
	if not tab.displayItem then
		error("no item is being viewed", 0)
	end
	-- auto-equip looks at which slots are shown
	synced_list()
	tab.controls.addDisplayItem.onClick()
end

-- The panel's Cancel button
function glue.display_cancel()
	local tab = items_tab()
	if not tab.displayItem then
		error("no item is being viewed", 0)
	end
	tab.controls.removeDisplayItem.onClick()
end

-- Ctrl+D's body (ItemsTab.lua:1481-1483); false when nothing changed
function glue.display_show_differences(on)
	local tab = items_tab()
	if (tab.showStatDifferences and true or false) == on then
		return false
	end
	tab.showStatDifferences = on
	pob.build.buildFlag = true
	return true
end

-- ---- The display item's editors (R11) ----
-- Each press first checks that PoB shows the control (and enables it, for a dropdown), then
-- runs the control's own callback.

local NOT_SHOWN = "PoB doesn't show that control for this item"

local function display_tab()
	local tab = items_tab()
	if not tab.displayItem then
		error("no item is being viewed", 0)
	end
	return tab
end

local function require_shown(control)
	if not (control and control:IsShown()) then
		error(NOT_SHOWN, 0)
	end
end

-- Picks entry `index` (1-based) of a dropdown that is shown and enabled. `SetSel` clamps, so
-- the range is checked first; picking the entry already selected does nothing, as in PoB.
local function pick_entry(control, index)
	require_shown(control)
	if not control:IsEnabled() then
		error(NOT_SHOWN, 0)
	end
	if index < 1 or index > #control.list then
		error("no such choice", 0)
	end
	control:SetSel(index)
end

local VARIANT_CONTROLS = {
	"displayItemVariant", "displayItemAltVariant", "displayItemAltVariant2",
	"displayItemAltVariant3", "displayItemAltVariant4", "displayItemAltVariant5",
}

-- `control` is 0..5; both the legacy dropdowns and the variant groups' ones call
-- SelectDisplayItemVariant
function glue.display_variant(control, index)
	local tab = display_tab()
	pick_entry(tab.controls[VARIANT_CONTROLS[control + 1]], index)
end

function glue.display_version(index)
	local tab = display_tab()
	pick_entry(tab.controls.displayItemVersion, index)
end

-- The boxes filter their text with "%D" and take two characters. Typing nothing that passes the
-- filter changes nothing.
function glue.display_quality(text)
	local tab = display_tab()
	local box = tab.controls.displayItemQualityEdit
	require_shown(box)
	glue.type_into(box, text)
end

function glue.display_catalyst_quality(text)
	local tab = display_tab()
	local box = tab.controls.displayItemCatalystQualityEdit
	require_shown(box)
	glue.type_into(box, text)
end

-- `which` is 0 or 1
function glue.display_influence(which, index)
	local tab = display_tab()
	local name = which == 0 and "displayItemInfluence" or which == 1 and "displayItemInfluence2"
	if not name then
		error(NOT_SHOWN, 0)
	end
	pick_entry(tab.controls[name], index)
end

function glue.display_catalyst(index)
	local tab = display_tab()
	pick_entry(tab.controls.displayItemCatalyst, index)
end

-- PoB syncs the socket controls to the item when it displays it and after adding a socket
function glue.display_socket(index, colour)
	local tab = display_tab()
	tab:UpdateSocketControls()
	local control = tab.controls["displayItemSocket" .. index]
	require_shown(control)
	for i, entry in ipairs(control.list) do
		if entry.color == colour then
			pick_entry(control, i)
			return
		end
	end
	error("no such colour", 0)
end

-- The check box after socket `index`: a click flips it, so pressing it for the state it has does
-- nothing
function glue.display_link(index, linked)
	local tab = display_tab()
	tab:UpdateSocketControls()
	local control = tab.controls["displayItemLink" .. index]
	require_shown(control)
	if (control.state and true or false) ~= linked then
		glue.press_check(control, linked)
	end
end

function glue.display_add_socket()
	local tab = display_tab()
	local button = tab.controls.displayItemAddSocket
	require_shown(button)
	button.onClick()
end

-- Whether the panel shows one stacked slider per line (the "show all affixes" option)
local function stacked(tab)
	return pob.main.main.showAllItemAffixes and tab.displayItem.rarity == "UNIQUE"
end

-- The slider (`want` "slider") or the Foulborn box ("box") of range line `line` (1-based), after
-- selecting the line in the range line dropdown as the user would. Whether PoB shows the control
-- is read from the line before anything moves, so a refusal leaves the selection as it was.
local function range_controls(tab, line, want)
	local mod_line = tab.displayItem.rangeLineList[line]
	if not mod_line then
		error("no such range line", 0)
	end
	local c = tab.controls
	if stacked(tab) then
		return c["displayItemStackedRangeSlider" .. line], c["displayItemStackedMutatedCheckbox" .. line]
	end
	local drop = c.displayItemRangeLine
	require_shown(drop)
	local present
	if want == "slider" then
		present = mod_line.showSlider
	else
		present = mod_line.modId and mod_line.newModId
	end
	if not present or line > #drop.list then
		error(NOT_SHOWN, 0)
	end
	if drop.selIndex ~= line then
		drop:SetSel(line)
	end
	return c.displayItemRangeSlider, c.displayItemMutatedCheckbox
end

-- `value` is the slider's 0..1. A slider's `val` is synced to the line only when its `shown` runs
-- in Draw (ItemsTab.lua:1076-1088), so IsShown runs first.
function glue.display_range(line, value)
	local tab = display_tab()
	local slider = range_controls(tab, line, "slider")
	require_shown(slider)
	slider:SetVal(value)
end

-- The box's state is likewise synced in `shown`; its click flips the line
function glue.display_mutated(line, on)
	local tab = display_tab()
	local _, box = range_controls(tab, line, "box")
	require_shown(box)
	if (box.state and true or false) ~= on then
		glue.press_check(box, on)
	end
end

local MOD_LINE_FIELDS = {
	enchant = "enchantModLines",
	scourge = "scourgeModLines",
	implicit = "implicitModLines",
	explicit = "explicitModLines",
	crucible = "crucibleModLines",
}

-- A click on a line of the display tooltip (ItemsTab.lua:1521-1539) calls
-- ToggleDisplayItemModLine; only a line that PoB's tooltip carries the mod line on is a click
function glue.display_toggle_mod(list, index)
	local tab = display_tab()
	local item = tab.displayItem
	local lines = item[MOD_LINE_FIELDS[list] or ""]
	local mod_line = lines and lines[index]
	if not mod_line then
		error("no such mod line", 0)
	end
	fresh_display_tooltip(tab)
	for _, line in ipairs(tab.displayItemTooltip.lines) do
		if line.modLine == mod_line then
			tab:ToggleDisplayItemModLine(mod_line)
			return
		end
	end
	error("that mod line isn't on the tooltip", 0)
end
