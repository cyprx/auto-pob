-- PoB's shared items and shared item sets (NF-2): `main.sharedItemList` and
-- `main.sharedItemSetList`, which PoB keeps in Settings.xml across builds. Every edit here is
-- the user's own and is saved at once through PoB's `main:SaveSettings` (R29, amended by the
-- controller: shared item list edits), which the override layer wraps (R12). The `<Mode>`
-- element is written as the file has it, so a shared edit changes only `<SharedItems>`.
--
-- The drags run PoB's own drop: the source list picks the row up as its Draw does, the target
-- is the control under the cursor, and the button's release is PoB's (ListControl.lua:146-176,
-- 423-452).

glue = glue or {}

local function main()
	return pob.main.main
end

local function items_tab()
	return pob.build.itemsTab
end

-- the draw-only updates the tab makes before a click acts (SP5a R16)
local function sync_tab()
	items_tab().controls.itemList:UpdateList()
	items_tab():UpdateSockets()
end

-- The shared list, which PoB shows only while no item is displayed (it hangs below "Craft
-- item...", ItemsTab.lua:320)
local function shared_list()
	local list = items_tab().controls.sharedItemList
	if not list:IsShown() then
		error("PoB doesn't offer that here", 0)
	end
	return list
end

local function shared_item(index)
	local item = main().sharedItemList[index]
	if not item then
		error("no such shared item", 0)
	end
	return item
end

-- ---- Saving ----

-- The `<Mode>` element of the Settings.xml on disk, or nil without a file
local function file_mode()
	local xml = common.xml.LoadXMLFile(main().userPath .. "Settings.xml")
	for _, node in ipairs(xml and xml[1] or {}) do
		if type(node) == "table" and node.elem == "Mode" then
			return node
		end
	end
end

-- PoB's `main:SaveSettings`, with `<Mode>` as the file has it: classic PoB writes the open
-- build there only at exit, and a shared edit is not an exit
local function save()
	local node = file_mode()
	if not node then
		main():SaveSettings()
		return
	end
	local args = {}
	for _, child in ipairs(node) do
		if type(child) == "table" and child.elem == "Arg" then
			local a = child.attrib
			if a.number then
				args[#args + 1] = tonumber(a.number)
			elseif a.boolean then
				args[#args + 1] = a.boolean == "true"
			else
				args[#args + 1] = a.string
			end
		end
	end
	local m = main()
	local saved_mode, own_call = m.mode, rawget(m, "CallMode")
	local call = m.CallMode
	m.mode = node.attrib.mode
	m.CallMode = function(self, name, ...)
		if name == "GetArgs" then
			return unpack(args, 1, #args)
		end
		return call(self, name, ...)
	end
	local ok, err = pcall(m.SaveSettings, m)
	m.mode = saved_mode
	m.CallMode = own_call
	if not ok then
		error(err, 0)
	end
end

-- Runs `fn` with PoB's popups closed afterwards, then saves when the lists changed
local function edit(fn)
	local function shape()
		local parts = {}
		for _, item in ipairs(main().sharedItemList) do
			parts[#parts + 1] = tostring(item)
		end
		for _, set in ipairs(main().sharedItemSetList) do
			parts[#parts + 1] = tostring(set) .. (set.title or "")
		end
		return table.concat(parts, ",")
	end
	local before = shape()
	glue.in_popups(fn)
	if shape() ~= before then
		save()
	end
end

-- ---- PoB's drop ----

-- Picks row `index` of `source` up, as the list's Draw starts a drag. `value` is the row's
-- value when the list isn't built (a database's shown list is filled as it draws, R15).
local function pick_up(source, index, value)
	if value ~= nil then
		source.selIndex, source.selValue = index, value
	else
		source:SelectIndex(index)
	end
	source.selDragging = true
	source.selDragActive = true
	source.dragType, source.dragValue = source:GetDragValue(source.selIndex, source.selValue)
end

local function put_down(source)
	for _, target in ipairs(source.dragTargetList or {}) do
		target.otherDragSource = nil
		target.otherDragTargeting = false
		target.selDragIndex = nil
	end
	source.selDragging = false
	source.selDragActive = false
	source.dragType, source.dragValue, source.dragTarget = nil, nil, nil
	source.selDragIndex = nil
end

-- Drags row `index` of `source` onto `target` and releases it there; `at` is the 1-based row
-- the target's own Draw would mark under the cursor (`selDragIndex`), or nil
local function drop(source, index, target, at, value)
	if not (source:IsShown() and source:IsEnabled()) then
		error("PoB doesn't offer that here", 0)
	end
	if not target:IsShown() then
		error("PoB doesn't offer that here", 0)
	end
	pick_up(source, index, value)
	if target.CanReceiveDrag and not target:CanReceiveDrag(source.dragType, source.dragValue) then
		put_down(source)
		error("that item can't go in that slot", 0)
	end
	target.otherDragSource = source
	target.selDragIndex = at
	source.dragTarget = target
	local ok, err = pcall(source.OnKeyUp, source, "LEFTBUTTON")
	put_down(source)
	target.selDragIndex = nil
	if not ok then
		error(err, 0)
	end
end

-- Drags row `from` of `list` to land at 1-based position `to` (ListControl.lua:427-436)
local function reorder(list, from, to)
	if not (list.list[from] and list.list[to]) then
		error("no such row", 0)
	end
	if from == to then
		return
	end
	pick_up(list, from)
	-- the drop row as Draw computes it: before the row under the cursor
	list.selDragIndex = to > from and to + 1 or to
	local ok, err = pcall(list.OnKeyUp, list, "LEFTBUTTON")
	put_down(list)
	if not ok then
		error(err, 0)
	end
end

-- ---- Shared items ----

-- Drags a build item (`kind` "item", `a` its id) or a database row (`kind` "db", `a` the
-- database, `b` the name) onto the shared list at 0-based row `at`
function glue.shared_add(kind, a, b, at)
	sync_tab()
	local target = shared_list()
	local count = #target.list
	if at < 0 or at > count then
		error("no such row", 0)
	end
	edit(function()
		if kind == "item" then
			local list = items_tab().controls.itemList
			local index
			for i, id in ipairs(list.list) do
				if id == a then
					index = i
				end
			end
			if not index then
				error("no such item", 0)
			end
			drop(list, index, target, at + 1)
		else
			local control, item, index = glue.item_db_find(a, b)
			drop(control, index, target, at + 1, item)
		end
	end)
end

-- Drags shared item `index` onto slot `slot_name`, or onto the build's item list (appended)
function glue.shared_drop(index, slot_name)
	sync_tab()
	local source = shared_list()
	shared_item(index)
	local target
	if slot_name then
		target = items_tab().slots[slot_name]
		if not target then
			error("no such slot", 0)
		end
		if not target:IsShown() then
			error("PoB doesn't offer that here", 0)
		end
	else
		target = items_tab().controls.itemList
	end
	-- it changes the build, not the shared list: nothing is saved
	glue.in_popups(function()
		drop(source, index, target, nil)
	end)
end

-- The Delete button and PoB's confirmation
function glue.shared_delete(index)
	sync_tab()
	local list = shared_list()
	local item = shared_item(index)
	edit(function()
		list:SelectIndex(index)
		list:OnSelDelete(index, item)
		glue.confirm_popup("remove the shared item")
	end)
end

function glue.shared_move(from, to)
	sync_tab()
	edit(function()
		reorder(shared_list(), from, to)
	end)
end

-- The double click: the item goes to the display panel
function glue.shared_view(index)
	sync_tab()
	local list = shared_list()
	local item = shared_item(index)
	list:OnSelClick(index, item, true)
end

-- ---- Shared item sets (the Manage Item Sets window) ----

-- Runs `fn(setList, sharedList)` on PoB's Manage Item Sets window, which is closed afterwards
local function with_set_manager(fn)
	sync_tab()
	local popups = main().popups
	local depth = #popups
	glue.in_popups(function()
		items_tab():OpenItemSetManagePopup()
		if #popups == depth then
			error("PoB didn't open that window", 0)
		end
		local controls = popups[1].controls
		fn(controls.setList, controls.sharedList)
	end)
end

local function shared_set(index)
	local set = main().sharedItemSetList[index]
	if not set then
		error("no such shared item set", 0)
	end
	return set
end

-- Drags the build's item set `id` onto the shared set list
function glue.shared_set_add(id)
	edit(function()
		with_set_manager(function(set_list, shared)
			local index
			for i, value in ipairs(set_list.list) do
				if value == id then
					index = i
				end
			end
			if not index then
				error("no such item set", 0)
			end
			drop(set_list, index, shared, nil)
		end)
	end)
end

-- Drags shared set `index` onto the build's set list (appended): a new item set
function glue.shared_set_import(index)
	shared_set(index)
	with_set_manager(function(set_list, shared)
		drop(shared, index, set_list, nil)
	end)
end

function glue.shared_set_delete(index)
	local set = shared_set(index)
	edit(function()
		with_set_manager(function(_, shared)
			shared:SelectIndex(index)
			shared:OnSelDelete(index, set)
			glue.confirm_popup("delete the shared item set")
		end)
	end)
end

-- The Rename button (or F2) and its name popup
function glue.shared_set_rename(index, title)
	local set = shared_set(index)
	edit(function()
		with_set_manager(function(_, shared)
			shared:SelectIndex(index)
			shared:RenameSet(set)
			local popup = main().popups[1]
			glue.type_into(popup.controls.edit, title)
			local save = popup.controls.save
			if not save:IsEnabled() then
				error("that name isn't allowed", 0)
			end
			save.onClick()
		end)
	end)
end

function glue.shared_set_move(from, to)
	edit(function()
		with_set_manager(function(_, shared)
			reorder(shared, from, to)
		end)
	end)
end

-- ---- Views ----

-- The shared lists as the Items view shows them, and whether PoB shows the shared list
function glue.shared_view_lists(tab)
	local list = tab.controls.sharedItemList
	local items = {}
	for i, item in ipairs(main().sharedItemList) do
		items[i] = { name = item.name, rarity = item.rarity, label = list:GetRowValue(1, i, item) }
	end
	-- PoB's set list is a control of the Manage Item Sets window; its rows and tooltips are
	-- read through one made for the purpose, never shown
	local sets = {}
	local set_list = new("SharedItemSetListControl", nil, { 0, 0, 300, 200 }, tab)
	for i, set in ipairs(main().sharedItemSetList) do
		local tooltip = new("Tooltip")
		set_list:AddValueTooltip(tooltip, i, set)
		local lines = {}
		for _, line in ipairs(tooltip.lines) do
			if line.text then
				lines[#lines + 1] = line.text
			end
		end
		sets[i] = { title = set_list:GetRowValue(1, i, set), lines = lines }
	end
	return items, sets, list:IsShown() and true or false
end

-- What hovering shared item `index` shows; nil when it is gone
function glue.shared_tooltip(index)
	local item = main().sharedItemList[index]
	if not item then
		return nil
	end
	local tooltip = new("Tooltip")
	items_tab().controls.sharedItemList:AddValueTooltip(tooltip, index, item)
	return glue.read_tooltip(tooltip)
end
