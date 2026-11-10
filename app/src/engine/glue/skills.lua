-- The Skills tab's state, as the tab's controls would show it.
-- PoB syncs the group controls with the displayed group in `SetDisplayGroup`, which also runs
-- `ProcessSocketGroup` and may rewrite gem levels. A view must not change the build, so each
-- group is shown by hand: the controls' closures only read `displayGroup` and the slot
-- dropdown's selection, so those two are set, the gem slots are updated, and both are put
-- back afterwards. Whether a control shows is `IsShown`, which also follows its anchors: a
-- gem's count and Vaal toggles hang off the delete button, which an item's group hides.

glue = glue or {}

local function drop_list(control, key_of)
	local labels = {}
	for i, item in ipairs(control.list) do
		labels[i] = type(item) == "table" and item.label or item
	end
	return labels
end

-- The 0-based index of the list entry whose `key` is `value`
local function index_by(control, key, value)
	for i, item in ipairs(control.list) do
		if type(item) == "table" and item[key] == value then return i - 1 end
	end
	return 0
end

local function drop_view(control, key, value)
	return {
		labels = drop_list(control),
		selected = index_by(control, key, value),
		enabled = control:GetProperty("enabled") ~= false,
	}
end

local function toggle_view(slot_control, gem, field)
	if not slot_control:IsShown() then return nil end
	return {
		label = slot_control:GetProperty("label"),
		on = gem[field] and true or false,
	}
end

local function gem_view(tab, group, index, gem)
	local slot = tab.gemSlots[index]
	local effect = (gem.gemData and gem.gemData.grantedEffect) or gem.grantedEffect
	return {
		name_spec = gem.nameSpec or "",
		gem_id = gem.gemId,
		level = gem.level,
		quality = gem.quality,
		enabled = gem.enabled and true or false,
		count = slot.count:IsShown() and (tonumber(gem.count) or 1) or nil,
		colour = gem.color or "",
		support = effect ~= nil and effect.support and true or false,
		matches_socket = gem.matchesSocket and true or false,
		error = gem.errMsg,
		toggle1 = toggle_view(slot.enableGlobal1, gem, "enableGlobal1"),
		toggle2 = toggle_view(slot.enableGlobal2, gem, "enableGlobal2"),
	}
end

local function group_view(tab, index, group)
	tab.displayGroup = group
	tab.controls.groupSlot:SelByValue(group.slot, "slotName")
	tab:UpdateGemSlots()
	local c = tab.controls
	local view = {
		label = group.label or "",
		display_label = c.groupList:GetRowValue(1, index, group),
		slot = group.slot,
		enabled = group.enabled and true or false,
		slot_enabled = group.slotEnabled ~= false,
		full_dps = (group.includeInFullDPS and group.enabled) and true or false,
		source = group.source,
		is_main = tab.build.mainSocketGroup == index,
		can_optimise = c.optimiseSockets:IsShown() and true or false,
		gems = {},
	}
	if c.groupCount:IsShown() then
		view.count = tostring(group.groupCount or 1)
	end
	if group.source ~= nil then
		view.source_note = c.sourceNote:GetProperty("label")
	end
	if c.imbuedSupportLabel:IsShown() then
		view.imbued = {
			name = group.imbuedSupport,
			enabled = c.imbuedSupport:GetProperty("enabled") and true or false,
		}
	end
	if c.socketsLabel:IsShown() then
		view.sockets_line = c.socketsLabel:GetProperty("label")
	end
	for i, gem in ipairs(group.gemList) do
		view.gems[i] = gem_view(tab, group, i, gem)
	end
	return view
end

local function options_view(tab)
	local c = tab.controls
	return {
		sort_by_dps = tab.sortGemsByDPS and true or false,
		sort_field = drop_view(c.sortGemsByDPSFieldControl, "type", tab.sortGemsByDPSField),
		default_level = drop_view(c.defaultLevel, "gemLevel", tab.defaultGemLevel),
		default_level_help = (function()
			local help = {}
			for i, item in ipairs(c.defaultLevel.list) do help[i] = item.description or "" end
			return help
		end)(),
		default_quality = tostring(tab.defaultGemQuality or 0),
		support_types = drop_view(c.showSupportGemTypes, "show", tab.showSupportGemTypes),
		show_legacy = tab.showLegacyGems and true or false,
	}
end

function glue.skills_view()
	local tab = pob.build.skillsTab
	local c = tab.controls
	local view = {
		sets = {},
		active_set = tab.activeSkillSetId,
		groups = {},
		slot_choices = {},
		options = options_view(tab),
		can_undo = tab.undo[2] ~= nil,
		can_redo = tab.redo[1] ~= nil,
	}
	for i, id in ipairs(tab.skillSetOrderList) do
		view.sets[i] = { id = id, title = tab.skillSets[id].title or "Default" }
	end
	for i, choice in ipairs(c.groupSlot.list) do
		view.slot_choices[i] = { label = choice.label, slot = choice.slotName }
	end
	local shown, slot_index = tab.displayGroup, c.groupSlot.selIndex
	local ok, err = pcall(function()
		for i, group in ipairs(tab.socketGroupList) do
			view.groups[i] = group_view(tab, i, group)
		end
	end)
	tab.displayGroup, c.groupSlot.selIndex = shown, slot_index
	-- the gem slots follow the group on show; make them match the restored one again
	tab:UpdateGemSlots()
	if not ok then error(err, 0) end
	return view
end

-- ---- Commands ----
-- Each one presses the control PoB's own tab would, after showing the group as a click on its
-- row does (`SetDisplayGroup`, which the controls' closures need). Indices are 1-based here.
-- Errors use level 0.

local function skills_tab()
	return pob.build.skillsTab
end

local function group_at(index)
	local group = skills_tab().socketGroupList[index]
	if not group then
		error("no such socket group", 0)
	end
	return group
end

-- A click on the group's row (SkillListControl:OnSelect)
local function show_group(index)
	local tab = skills_tab()
	local group = group_at(index)
	local list = tab.controls.groupList
	list.selIndex, list.selValue = index, group
	tab:SetDisplayGroup(group)
	return group, tab.controls
end
glue.skills_show_group = show_group

-- Runs `fn`, then closes every popup it left open, also when it fails.
local function in_popups(fn)
	local popups = pob.main.main.popups
	local depth = #popups
	local ok, err = pcall(fn)
	while #popups > depth do
		pob.main.main:ClosePopup()
	end
	if not ok then
		error(err, 0)
	end
end

-- Presses the confirm button of the newest popup
local function confirm_popup(what)
	local popup = pob.main.main.popups[1]
	local button = popup and popup.controls.confirm
	if not (button and button.onClick) then
		error("couldn't " .. what, 0)
	end
	button.onClick()
end

-- SkillListControl's New button (SkillListControl.lua:38-52)
function glue.skills_add_group()
	skills_tab().controls.groupList.controls.new.onClick()
end

-- OnSelDelete and its confirmation (SkillListControl.lua:150-186)
function glue.skills_delete_group(index)
	local group = group_at(index)
	if group.source then
		error("this socket group is created by an equipped item and can't be deleted", 0)
	end
	local list = skills_tab().controls.groupList
	list.selIndex, list.selValue = index, group
	in_popups(function()
		local popups = #pob.main.main.popups
		list:OnSelDelete(index, group)
		-- a group with no gems goes without asking
		if #pob.main.main.popups > popups then
			confirm_popup("delete the socket group")
		end
	end)
end

-- The Delete All button and its confirmation (SkillListControl.lua:38-52)
function glue.skills_delete_all()
	local list = skills_tab().controls.groupList
	if #list.list == 0 then
		return
	end
	in_popups(function()
		list.controls.deleteAll.onClick()
		confirm_popup("delete the socket groups")
	end)
end

-- ListControl's drop (ListControl.lua:426-436); `to` is the group's final position
function glue.skills_move_group(from, to)
	local list = skills_tab().controls.groupList
	local group = group_at(from)
	group_at(to)
	if from == to then
		return
	end
	list.selIndex, list.selValue = from, group
	-- the drop's own index is adjusted for the removal, which leaves `to`
	table.remove(list.list, from)
	table.insert(list.list, to, group)
	list:OnOrderChange(from, to)
	list.selValue = nil
end

-- A right-click on a group's row, "Set as main skill" (SkillListControl.lua:210-215); undoable,
-- unlike the sidebar's dropdown
function glue.skills_set_main(index)
	group_at(index)
	local tab = skills_tab()
	tab.build.mainSocketGroup = index
	tab:AddUndoState()
	tab.build.buildFlag = true
end

-- Ctrl+V on the tab (SkillsTab.lua:667). Paste() is empty headless, so `testInput` is the text
function glue.skills_paste(text)
	local tab = skills_tab()
	local before = #tab.socketGroupList
	tab:PasteSocketGroup(text)
	if #tab.socketGroupList == before then
		error("the text has no gems", 0)
	end
end

-- Types into an edit control as PoB's EditControl would (R3)
local function type_into(control, text)
	text = text:gsub("\r", "")
	local typed = text
	text = text:gsub(control.filterPattern, "")
	-- everything was filtered out: EditControl:Insert does nothing
	if typed ~= "" and text == "" then
		return false
	end
	if control.limit and #text > control.limit then
		error("the text is too long", 0)
	end
	control:SetText(text, true)
	return true
end

function glue.skills_set_label(index, text)
	local _, c = show_group(index)
	type_into(c.groupLabel, text)
end

-- The slot dropdown (SkillsTab.lua:168); `slot` is nil for "None"
function glue.skills_set_slot(index, slot)
	local group, c = show_group(index)
	if group.source then
		error("the slot of an item's socket group can't be changed", 0)
	end
	for i, choice in ipairs(c.groupSlot.list) do
		if choice.slotName == slot then
			c.groupSlot:SetSel(i)
			return
		end
	end
	error("unknown slot " .. tostring(slot), 0)
end

local function press_check(control, on)
	control.state = on
	control.changeFunc(on)
end

function glue.skills_set_enabled(index, on)
	local _, c = show_group(index)
	press_check(c.groupEnabled, on)
end

function glue.skills_set_full_dps(index, on)
	local _, c = show_group(index)
	press_check(c.includeInFullDPS, on)
end

-- SkillListControl:OnSelCopy -> SkillsTab:CopySocketGroup (SkillsTab.lua:667-678). The global
-- `Copy` is swapped for a capture during the call and always put back; nothing reaches the
-- system clipboard.
function glue.skills_copy(index)
	local group = group_at(index)
	if group.source then
		error("an item's socket group can't be copied", 0)
	end
	local saved, text = Copy, nil
	Copy = function(copied)
		text = copied
	end
	local ok, err = pcall(skills_tab().CopySocketGroup, skills_tab(), group)
	Copy = saved
	if not ok then
		error(err, 0)
	end
	return text or ""
end

-- Ctrl+Z and Ctrl+Y on the tab (SkillsTab.lua:622-627)
function glue.skills_undo()
	skills_tab():Undo()
	pob.build.buildFlag = true
end

function glue.skills_redo()
	skills_tab():Redo()
	pob.build.buildFlag = true
end

-- ---- Gem slots ----

-- The gem slot control set for an existing gem (R4); `append` also allows the empty row
local function gem_slot(group_index, slot_index, append)
	local group, c = show_group(group_index)
	local tab = skills_tab()
	local count = #group.gemList
	if slot_index < 1 or slot_index > count + (append and 1 or 0) then
		error("no such gem", 0)
	end
	if group.source ~= nil then
		error("an item's socket group can't be edited", 0)
	end
	return group, tab.gemSlots[slot_index], c
end

-- Enter in the gem's name box with `gem_id` highlighted (GemSelectControl.lua:763-773, R5).
-- The control must offer the gem, so a hidden one can't make PoB delete the slot's gem.
local function pick(control, gem_id)
	-- A search's S or A filter (`sortGemsBy`) hides gems from `control.gems`. A click on the
	-- box body clears it (GemSelectControl.lua:732-741), and UpdateSortCache repopulates the
	-- gems with it, so `gems` and `lastSortGemsBy` keep agreeing for the next search.
	if control.sortGemsBy ~= nil or control.lastSortGemsBy ~= nil then
		control.sortGemsBy = nil
		control:UpdateSortCache()
	end
	control:PopulateGemList()
	-- the imbued box offers what can support the group's skill (R5)
	if control.imbuedSelect then
		control:UpdateSortCache()
	end
	local key = "Default:" .. gem_id
	local data = control.gems[key]
	if not data or not control:FilterSupport(key, data) then
		error("that gem isn't offered here", 0)
	end
	control.initialBuf = control.buf
	control.list = { key }
	control.selIndex = 1
	control:SetText(data.name)
	control:UpdateGem(true, true, true)
	control.dropped = false
end

function glue.skills_set_gem(group_index, slot_index, gem_id)
	local _, slot = gem_slot(group_index, slot_index, true)
	pick(slot.nameSpec, gem_id)
end

function glue.skills_set_gem_level(group_index, slot_index, text)
	local _, slot = gem_slot(group_index, slot_index)
	type_into(slot.level, text)
end

function glue.skills_set_gem_quality(group_index, slot_index, text)
	local _, slot = gem_slot(group_index, slot_index)
	type_into(slot.quality, text)
end

function glue.skills_set_gem_count(group_index, slot_index, text)
	local _, slot = gem_slot(group_index, slot_index)
	if not slot.count:IsShown() then
		error("this gem has no count", 0)
	end
	type_into(slot.count, text)
end

function glue.skills_set_gem_enabled(group_index, slot_index, on)
	local group, slot = gem_slot(group_index, slot_index)
	-- PoB's callback reads `gemData.vaalGem` first and would raise on a gem it doesn't know
	if not group.gemList[slot_index].gemData then
		error("the gem isn't in PoB's data", 0)
	end
	press_check(slot.enabled, on)
end

function glue.skills_set_gem_toggle(group_index, slot_index, which, on)
	local _, slot = gem_slot(group_index, slot_index)
	local control = which == 1 and slot.enableGlobal1 or slot.enableGlobal2
	if not control:IsShown() then
		error("this gem has no such toggle", 0)
	end
	press_check(control, on)
end

function glue.skills_delete_gem(group_index, slot_index)
	local _, slot = gem_slot(group_index, slot_index)
	slot.delete.onClick()
end

-- ---- Skill sets (R16) ----

local function set_index(id)
	for i, set_id in ipairs(skills_tab().skillSetOrderList) do
		if set_id == id then
			return i
		end
	end
	error("no such skill set", 0)
end

-- Opens the Manage Skill Sets popup (SkillsTab.lua:1415), calls `fn(list)` with its
-- SkillSetListControl and closes what is left open.
local function with_set_list(fn)
	in_popups(function()
		skills_tab():OpenSkillSetManagePopup()
		for _, control in pairs(pob.main.main.popups[1].controls) do
			if control.controls and control.controls.new and control.OnSelDelete then
				return fn(control)
			end
		end
		error("couldn't open the skill set list", 0)
	end)
end

local function check_title(title)
	if not title:gsub("\r", ""):match("%S") then
		error("the skill set needs a name", 0)
	end
end

-- Types the title into the name popup `RenameSet` opened and presses Save (Save stays disabled
-- until the name has a visible character). When that fails, Cancel is pressed, which drops a
-- set that New or Copy had already made.
local function save_name(title)
	local popup = pob.main.main.popups[1]
	local ok, applied = pcall(type_into, popup.controls.edit, title)
	if ok and applied and popup.controls.save.enabled then
		popup.controls.save.onClick()
		return
	end
	popup.controls.cancel.onClick()
	if not ok then
		error(applied, 0)
	end
	-- the filter emptied the title, or Save stayed disabled: either way it is blank
	check_title("")
end

-- Draw's sync of the set dropdown (SkillsTab.lua:646-654), then the user's pick
function glue.skills_select_set(id)
	local tab = skills_tab()
	local index = set_index(id)
	local select = tab.controls.setSelect
	local titles = {}
	for i, set_id in ipairs(tab.skillSetOrderList) do
		titles[i] = tab.skillSets[set_id].title or "Default"
		if set_id == tab.activeSkillSetId then
			select.selIndex = i
		end
	end
	select:SetList(titles)
	select:SetSel(index)
end

-- The New button and the name popup (SkillSetListControl.lua:41-43, 36-58)
function glue.skills_new_set(title)
	check_title(title)
	with_set_list(function(list)
		list.controls.new.onClick()
		save_name(title)
	end)
end

-- The Rename button and the name popup
function glue.skills_rename_set(id, title)
	set_index(id)
	check_title(title)
	with_set_list(function(list)
		list:RenameSet(skills_tab().skillSets[id])
		save_name(title)
	end)
	glue.frame()
end

-- The Copy button and the name popup (SkillSetListControl.lua:11-29)
function glue.skills_copy_set(id, title)
	local index = set_index(id)
	check_title(title)
	with_set_list(function(list)
		list.selIndex, list.selValue = index, id
		list.controls.copy.onClick()
		save_name(title)
	end)
end

-- OnSelDelete and its confirmation (SkillSetListControl.lua:84-99)
function glue.skills_delete_set(id)
	local index = set_index(id)
	if #skills_tab().skillSetOrderList <= 1 then
		error("the last skill set can't be deleted", 0)
	end
	with_set_list(function(list)
		list.selIndex, list.selValue = index, id
		list:OnSelDelete(index, id)
		confirm_popup("delete the skill set")
	end)
end

-- ---- Gem options (R18) ----

function glue.skills_option_check(name, on)
	press_check(skills_tab().controls[name], on)
end

function glue.skills_option_drop(name, index)
	local control = skills_tab().controls[name]
	if index < 1 or index > #control.list then
		error("no such option", 0)
	end
	control:SetSel(index)
end

function glue.skills_option_quality(text)
	type_into(skills_tab().controls.defaultQuality, text)
end

-- ---- Item groups, imbued supports, Optimise Sockets and tooltips ----

-- The Count box, which only an item's group shows (SkillsTab.lua:345-356)
function glue.skills_set_group_count(index, text)
	local group, c = show_group(index)
	if not c.groupCount:IsShown() then
		error("only a socket group an item grants has a count", 0)
	end
	type_into(c.groupCount, text)
end

-- The Imbued Support box and its x button (SkillsTab.lua:289-343); `gem_id` is nil to clear
function glue.skills_set_imbued(index, gem_id)
	local group, c = show_group(index)
	if not c.imbuedSupportLabel:IsShown() then
		error("an item's socket group has no imbued support", 0)
	end
	if not c.imbuedSupport:GetProperty("enabled") then
		if not group.slot then
			error("the socket group needs an item slot to hold an imbued support", 0)
		end
		error("the imbued support of this slot can't be changed", 0)
	end
	if gem_id == nil then
		c.imbuedSupportClear.onClick()
	else
		pick(c.imbuedSupport, gem_id)
	end
end

-- The Optimise Sockets button (SkillsTab.lua:242-288); it records an Items undo state (R20)
function glue.skills_optimise(index)
	local _, c = show_group(index)
	if not c.optimiseSockets:IsShown() then
		error("this socket group has no item with sockets to optimise", 0)
	end
	c.optimiseSockets.onClick()
end

-- What the group list shows when the mouse is over a group (R15). The reply for a group that
-- is gone, or a failing tooltip, is no lines.
function glue.skills_group_tooltip(index)
	local tab = skills_tab()
	local group = tab.socketGroupList[index]
	local tooltip = glue.tooltip()
	if group then
		local ok, built = pcall(function()
			local recorder = glue.tooltip()
			tab.controls.groupList:AddValueTooltip(recorder, index, group)
			return recorder
		end)
		if ok then tooltip = built end
	end
	return { group = index - 1, lines = tooltip:lines() }
end
