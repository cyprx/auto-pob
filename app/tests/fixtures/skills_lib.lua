-- Helpers for the Skills recipes. Runs in unmodified upstream PoB after a build is loaded.
-- Written independently of glue/skills.lua: each group goes through PoB's own
-- `SetDisplayGroup`, and the values are read from the controls it syncs. Nothing here may call
-- our glue.

local function shown(control)
	return control:IsShown() and true or false
end

local function labels(control)
	local out = {}
	for i, item in ipairs(control.list) do
		out[i] = type(item) == "table" and item.label or item
	end
	return out
end

local function dropdown(control, selected)
	return { labels = labels(control), selected = selected - 1, enabled = control:IsEnabled() and true or false }
end

local function selIndexByValue(control, key, value)
	for i, item in ipairs(control.list) do
		if item[key] == value then return i end
	end
	return 1
end

local function toggle(control)
	if not shown(control) then return nil end
	return { label = control:GetProperty("label"), on = control.state and true or false }
end

local function groupDump(tab, index, group)
	tab:SetDisplayGroup(group)
	local c = tab.controls
	local out = {
		label = c.groupLabel.buf,
		display_label = c.groupList:GetRowValue(1, index, group),
		slot = c.groupSlot:GetSelValue().slotName,
		enabled = c.groupEnabled.state and true or false,
		slot_enabled = group.slotEnabled ~= false,
		full_dps = c.includeInFullDPS.state and true or false,
		source = group.source,
		is_main = build.mainSocketGroup == index,
		can_optimise = shown(c.optimiseSockets),
		gems = {},
	}
	if shown(c.groupCount) then out.count = c.groupCount.buf end
	if group.source ~= nil then out.source_note = c.sourceNote.label() end
	if shown(c.imbuedSupportLabel) then
		out.imbued = { name = group.imbuedSupport, enabled = c.imbuedSupport:IsEnabled() and true or false }
	end
	if shown(c.socketsLabel) then out.sockets_line = c.socketsLabel.label() end
	for i, gem in ipairs(group.gemList) do
		local slot = tab.gemSlots[i]
		local effect = gem.gemData and gem.gemData.grantedEffect or gem.grantedEffect
		out.gems[i] = {
			name_spec = slot.nameSpec.buf,
			gem_id = gem.gemId,
			level = tonumber(slot.level.buf),
			quality = tonumber(slot.quality.buf),
			enabled = slot.enabled.state and true or false,
			count = shown(slot.count) and tonumber(slot.count.buf) or nil,
			colour = gem.color,
			support = effect ~= nil and effect.support and true or false,
			matches_socket = gem.matchesSocket and true or false,
			error = gem.errMsg,
			toggle1 = toggle(slot.enableGlobal1),
			toggle2 = toggle(slot.enableGlobal2),
		}
	end
	return out
end

-- The Skills tab as plain tables
function skillsDump()
	local tab = build.skillsTab
	local c = tab.controls
	local out = {
		sets = {}, active_set = tab.activeSkillSetId, groups = {}, slot_choices = {},
		can_undo = tab.undo[2] ~= nil, can_redo = tab.redo[1] ~= nil,
	}
	for i, id in ipairs(tab.skillSetOrderList) do
		out.sets[i] = { id = id, title = tab.skillSets[id].title or "Default", untitled = tab.skillSets[id].title == nil }
	end
	for i, choice in ipairs(c.groupSlot.list) do
		out.slot_choices[i] = { label = choice.label, slot = choice.slotName }
	end
	for i, group in ipairs(tab.socketGroupList) do
		out.groups[i] = groupDump(tab, i, group)
	end
	local help = {}
	for i, item in ipairs(c.defaultLevel.list) do help[i] = item.description end
	out.options = {
		sort_by_dps = tab.sortGemsByDPS and true or false,
		sort_field = dropdown(c.sortGemsByDPSFieldControl,
			selIndexByValue(c.sortGemsByDPSFieldControl, "type", tab.sortGemsByDPSField)),
		default_level = dropdown(c.defaultLevel, selIndexByValue(c.defaultLevel, "gemLevel", tab.defaultGemLevel)),
		default_level_help = help,
		default_quality = tostring(tab.defaultGemQuality or 0),
		support_types = dropdown(c.showSupportGemTypes,
			selIndexByValue(c.showSupportGemTypes, "show", tab.showSupportGemTypes)),
		show_legacy = tab.showLegacyGems and true or false,
	}
	return out
end

-- The sidebar's main-skill selector as plain tables, read from the build's own controls. The
-- build runs `RefreshSkillSelectControls` while it draws, so call this after a frame.
function mainSkillDump()
	local c = build.controls
	local function drop(name)
		local control = c[name]
		if not control.shown then return nil end
		return dropdown(control, control.selIndex or 1)
	end
	local function text(name)
		local control = c[name]
		if not control.shown then return nil end
		return control.buf or ""
	end
	return {
		group = dropdown(c.mainSocketGroup, c.mainSocketGroup.selIndex or 1),
		skill = drop("mainSkill"),
		part = drop("mainSkillPart"),
		stages = text("mainSkillStageCount"),
		mines = text("mainSkillMineCount"),
		minion = drop("mainSkillMinion"),
		minion_skill = drop("mainSkillMinionSkill"),
	}
end

-- ---- Recipe actions ----
-- Every `act.*` presses PoB's own control the way the tab does, records the command for the
-- gate to replay (`cmd` is our command's name, indices are 0-based), then runs a frame.
-- Nothing here may call our glue.

actions = {}
act = {}

local function frame()
	runCallback("OnFrame")
end

local function skillsTab()
	return build.skillsTab
end

local function groupAt(index)
	local group = skillsTab().socketGroupList[index + 1]
	assert(group, "no socket group " .. index)
	return group
end

-- A click on the group's row: selects it and shows it (SkillListControl.lua:116-118)
local function clickRow(index)
	local list = skillsTab().controls.groupList
	local group = groupAt(index)
	list.selIndex, list.selValue = index + 1, group
	list:OnSelect(index + 1, group)
	return group, skillsTab().controls
end

-- Presses the `confirm` button of the newest popup
local function confirm()
	local popup = main.popups[1]
	assert(popup, "no popup is open")
	popup.controls.confirm.onClick()
end

local function record(entry)
	table.insert(actions, entry)
	frame()
end

-- The New button (SkillListControl.lua:38-52)
function act.add_group()
	skillsTab().controls.groupList.controls.new.onClick()
	record({ cmd = "AddSocketGroup" })
end

-- The Delete button and its confirmation (SkillListControl.lua:23-26, 150-186)
function act.delete_group(index)
	clickRow(index)
	local list = skillsTab().controls.groupList
	local hasGems = groupAt(index).gemList[1] ~= nil
	list.controls.delete.onClick()
	if hasGems then confirm() end
	record({ cmd = "DeleteSocketGroup", group = index })
end

-- The Delete All button and its confirmation (SkillListControl.lua:27-41)
function act.delete_all()
	skillsTab().controls.groupList.controls.deleteAll.onClick()
	confirm()
	record({ cmd = "DeleteAllSocketGroups" })
end

-- Dragging the row to a new place and dropping it (ListControl.lua:426-436); `to` is the
-- final position, which a drop expresses as the row it lands before.
function act.move_group(from, to)
	local list = skillsTab().controls.groupList
	list.selIndex, list.selValue = from + 1, groupAt(from)
	local dragIndex = to > from and to + 2 or to + 1
	table.remove(list.list, list.selIndex)
	if dragIndex > list.selIndex then
		dragIndex = dragIndex - 1
	end
	table.insert(list.list, dragIndex, list.selValue)
	list:OnOrderChange(list.selIndex, dragIndex)
	list.selValue = nil
	record({ cmd = "MoveSocketGroup", from = from, to = to })
end

-- Ctrl+V on the tab (SkillsTab.lua:632-634); Paste() is empty headless, so the text is the
-- test input
function act.paste(text)
	skillsTab():PasteSocketGroup(text)
	record({ cmd = "PasteSocketGroup", text = text })
end

-- Typing into the label box (SkillsTab.lua:157-162)
function act.label(index, text)
	local _, c = clickRow(index)
	c.groupLabel:SetText(text, true)
	record({ cmd = "SetGroupLabel", group = index, text = text })
end

-- The slot dropdown (SkillsTab.lua:168-184)
function act.slot(index, name)
	local _, c = clickRow(index)
	local found
	for i, choice in ipairs(c.groupSlot.list) do
		if choice.slotName == name then
			c.groupSlot:SetSel(i)
			found = true
		end
	end
	assert(found, "no slot " .. tostring(name))
	record({ cmd = "SetGroupSlot", group = index, slot = name })
end

-- The Enabled and Include in Full DPS checkboxes (SkillsTab.lua:204-214)
function act.enable(index, on)
	local _, c = clickRow(index)
	c.groupEnabled.state = on
	c.groupEnabled.changeFunc(on)
	record({ cmd = "SetGroupEnabled", group = index, on = on })
end

function act.full_dps(index, on)
	local _, c = clickRow(index)
	c.includeInFullDPS.state = on
	c.includeInFullDPS.changeFunc(on)
	record({ cmd = "SetGroupFullDps", group = index, on = on })
end

-- The gem slot controls of an existing gem (or the empty row when `empty`), after a click on
-- the group's row has shown the group
local function gemSlot(g, s, empty)
	local group, c = clickRow(g)
	assert(s + 1 <= #group.gemList + (empty and 1 or 0), "no gem " .. s)
	return group, skillsTab().gemSlots[s + 1], c
end

-- Typing a gem's name into its box and pressing Enter (GemSelectControl.lua:699-706, 722-773):
-- a click on the box body, focus, an empty box, the name typed (whose changeFunc builds the list and previews the
-- gem), then the RETURN key. `text` is the gem's name as the list shows it.
function act.gem(g, s, name)
	local group, slot = gemSlot(g, s, true)
	local control = slot.nameSpec
	-- the click lands on the box body, which clears an S or A filter (GemSelectControl.lua:739)
	control.sortGemsBy = nil
	control:OnFocusGained()
	control:SetText("")
	control:Insert(name)
	control:OnKeyDown("RETURN")
	control.dropped = false
	local gem = group.gemList[s + 1]
	assert(gem and gem.gemId, "no gem was picked for " .. name)
	record({ cmd = "SetGem", group = g, slot = s, gem_id = gem.gemId })
end

-- The level, quality and count boxes (SkillsTab.lua:812-1000)
local function typeInto(field, cmd)
	return function(g, s, text)
		local _, slot = gemSlot(g, s)
		slot[field]:SetText(text, true)
		record({ cmd = cmd, group = g, slot = s, text = text })
	end
end
act.level = typeInto("level", "SetGemLevel")
act.quality = typeInto("quality", "SetGemQuality")
act.count = typeInto("count", "SetGemCount")

-- The gem's enabled checkbox
function act.gem_enabled(g, s, on)
	local _, slot = gemSlot(g, s)
	slot.enabled.state = on
	slot.enabled.changeFunc(on)
	record({ cmd = "SetGemEnabled", group = g, slot = s, on = on })
end

-- A Vaal gem's "Enable <skill>" checkboxes; `which` is 1 or 2
function act.toggle(g, s, which, on)
	local _, slot = gemSlot(g, s)
	local box = slot["enableGlobal" .. which]
	box.state = on
	box.changeFunc(on)
	record({ cmd = "SetGemToggle", group = g, slot = s, which = which == 1 and "First" or "Second", on = on })
end

-- The x button beside the gem
function act.delete_gem(g, s)
	local _, slot = gemSlot(g, s)
	slot.delete.onClick()
	record({ cmd = "DeleteGem", group = g, slot = s })
end

-- The item group's Count box (SkillsTab.lua:345-356)
function act.group_count(index, text)
	local _, c = clickRow(index)
	c.groupCount:SetText(text, true)
	record({ cmd = "SetGroupCount", group = index, text = text })
end

-- Typing the gem's name into the Imbued Support box and pressing Enter, as `act.gem` does
function act.imbued(index, name)
	local group, c = clickRow(index)
	local control = c.imbuedSupport
	control:OnFocusGained()
	control:SetText("")
	control:Insert(name)
	control:OnKeyDown("RETURN")
	control.dropped = false
	assert(group.imbuedSupport, "no imbued support was picked for " .. name)
	local id
	for key, gem in pairs(data.gems) do
		if gem.name == group.imbuedSupport then id = key end
	end
	assert(id, "no gem is called " .. group.imbuedSupport)
	record({ cmd = "SetImbuedSupport", group = index, gem_id = id })
end

-- The x button beside the Imbued Support box
function act.imbued_clear(index)
	local _, c = clickRow(index)
	c.imbuedSupportClear.onClick()
	record({ cmd = "SetImbuedSupport", group = index, gem_id = nil })
end

-- The Optimise Sockets button
function act.optimise(index)
	local _, c = clickRow(index)
	c.optimiseSockets.onClick()
	record({ cmd = "OptimiseSockets", group = index })
end

-- ---- Skill sets ----
-- Sets are addressed by PoB's id, as our commands do.

-- Opens the Manage Skill Sets popup (SkillsTab.lua:1415), calls `fn(list)` with its
-- SkillSetListControl and closes what is left open.
local function withSetList(fn)
	local depth = #main.popups
	skillsTab():OpenSkillSetManagePopup()
	local list
	for _, control in pairs(main.popups[1].controls) do
		if control.controls and control.controls.new and control.OnSelDelete then list = control end
	end
	assert(list, "no skill set list")
	fn(list)
	while #main.popups > depth do main:ClosePopup() end
end

local function setIndex(id)
	for i, setId in ipairs(skillsTab().skillSetOrderList) do
		if setId == id then return i end
	end
	error("no skill set " .. tostring(id))
end

-- Types the title into the name popup on top and presses Save (SkillSetListControl.lua:36-58)
local function saveName(title)
	local popup = main.popups[1]
	popup.controls.edit:SetText(title, true)
	assert(popup.controls.save.enabled, "Save is disabled")
	popup.controls.save.onClick()
end

-- The New button and the name popup (SkillSetListControl.lua:41-43)
function act.new_set(title)
	withSetList(function(list)
		list.controls.new.onClick()
		saveName(title)
	end)
	record({ cmd = "NewSkillSet", title = title })
end

-- The Copy button and the name popup (SkillSetListControl.lua:11-29)
function act.copy_set(id, title)
	withSetList(function(list)
		list.selIndex, list.selValue = setIndex(id), id
		list.controls.copy.onClick()
		saveName(title)
	end)
	record({ cmd = "CopySkillSet", id = id, title = title })
end

-- The Rename button and the name popup (SkillSetListControl.lua:30-35)
function act.rename_set(id, title)
	setIndex(id)
	withSetList(function(list)
		list:RenameSet(skillsTab().skillSets[id])
		saveName(title)
	end)
	record({ cmd = "RenameSkillSet", id = id, title = title })
end

-- OnSelDelete and its confirmation (SkillSetListControl.lua:84-99)
function act.delete_set(id)
	withSetList(function(list)
		local index = setIndex(id)
		list.selIndex, list.selValue = index, id
		list:OnSelDelete(index, id)
		confirm()
	end)
	record({ cmd = "DeleteSkillSet", id = id })
end

-- The set dropdown (SkillsTab.lua:94-97), after Draw has put the titles in it
function act.select_set(id)
	local tab = skillsTab()
	local select = tab.controls.setSelect
	local titles = {}
	for i, setId in ipairs(tab.skillSetOrderList) do
		titles[i] = tab.skillSets[setId].title or "Default"
	end
	select:SetList(titles)
	select.selIndex = setIndex(tab.activeSkillSetId)
	select:SetSel(setIndex(id))
	record({ cmd = "SelectSkillSet", id = id })
end

-- ---- Gem options ----
-- `name` is one of sort_by_dps, sort_field, default_level, default_quality, support_types and
-- show_legacy; dropdown values are 0-based indices, the quality is the text typed.
function act.option(name, value)
	local c = skillsTab().controls
	local checks = { sort_by_dps = c.sortGemsByDPS, show_legacy = c.showLegacyGems }
	local drops = { sort_field = c.sortGemsByDPSFieldControl, default_level = c.defaultLevel,
		support_types = c.showSupportGemTypes }
	if checks[name] then
		checks[name].state = value
		checks[name].changeFunc(value)
	elseif drops[name] then
		drops[name]:SetSel(value + 1)
	elseif name == "default_quality" then
		c.defaultQuality:SetText(value, true)
	else
		error("unknown option " .. tostring(name))
	end
	record({ cmd = "SetSkillOption", option = name, value = value })
end

-- A right-click on the group's row, "Set as main skill" (SkillListControl.lua:197-205). The
-- row's hover is faked, as no mouse is there.
function act.main_group(index)
	local list = skillsTab().controls.groupList
	local group = groupAt(index)
	local class = list.ListControl
	local saved = { class.GetHoverIndex, class.GetHoverValue }
	class.GetHoverIndex = function() return index + 1 end
	class.GetHoverValue = function() return group end
	local ok, err = pcall(function() list:OnHoverKeyUp("RIGHTBUTTON") end)
	class.GetHoverIndex, class.GetHoverValue = saved[1], saved[2]
	if not ok then error(err, 0) end
	record({ cmd = "SetMainSocketGroup", group = index })
end

-- A sidebar dropdown (Build.lua:474-561): `name` is the control, `index` is 0-based. The
-- build refreshes its dropdowns while it draws, so a frame runs first.
local sidebarNames = {
	mainSocketGroup = "Group", mainSkill = "Skill", mainSkillPart = "Part",
	mainSkillMinion = "Minion", mainSkillMinionSkill = "MinionSkill",
}
function act.sidebar(name, index)
	frame()
	local control = build.controls[name]
	assert(sidebarNames[name] and control.shown, "the sidebar doesn't show " .. tostring(name))
	control:SetSel(index + 1)
	record({ cmd = "SetMainSkillSelect", control = sidebarNames[name], index = index })
end

-- The sidebar's Stages and Active Mines boxes
local countNames = { mainSkillStageCount = "Stages", mainSkillMineCount = "Mines" }
function act.sidebar_count(name, text)
	frame()
	local control = build.controls[name]
	assert(countNames[name] and control.shown, "the sidebar doesn't show " .. tostring(name))
	-- typing: the box drops what its filter refuses
	control:SetText("")
	control:Insert(text)
	record({ cmd = "SetMainSkillCount", control = countNames[name], text = text })
end

-- Ctrl+Z and Ctrl+Y on the tab (SkillsTab.lua:622-627)
function act.undo()
	skillsTab():Undo()
	build.buildFlag = true
	record({ cmd = "Undo" })
end

function act.redo()
	skillsTab():Redo()
	build.buildFlag = true
	record({ cmd = "Redo" })
end

-- What Ctrl+C on the group's row copies (SkillListControl.lua:143-147), captured instead of
-- reaching the clipboard; nil for a group that can't be copied
function copyText(index)
	local text
	local saved = Copy
	Copy = function(copied) text = copied end
	local ok, err = pcall(function()
		skillsTab().controls.groupList:OnSelCopy(index + 1, groupAt(index))
	end)
	Copy = saved
	if not ok then error(err, 0) end
	return text
end

-- The Skills tab right now, recorded for the gate to compare with ours at this point. The
-- host defines `sidebar_lines()`.
function checkpoint()
	frame()
	local dump = skillsDump()
	table.insert(actions, { check = {
		sets = dump.sets, active_set = dump.active_set, groups = dump.groups,
		can_undo = dump.can_undo, can_redo = dump.can_redo, sidebar = sidebar_lines(),
		main_skill = mainSkillDump(),
	} })
end

-- ---- Gem search and tooltips (R23) ----

-- The gem box `target` names: a 0-based gem slot, or "imbued" for the Imbued Support box
local function gemBox(g, target)
	local _, c = clickRow(g)
	if target == "imbued" then
		return c.imbuedSupport
	end
	local slot = skillsTab().gemSlots[target + 1]
	assert(slot, "no gem slot " .. tostring(target))
	return slot.nameSpec
end

local viewport = { x = 0, y = 0, width = 1920, height = 1080 }

-- Searching in a gem box as a user does: a click on the box (the S or A overlay sets
-- `sortGemsBy`, GemSelectControl.lua:732-741) focuses it, which drops the list and starts the
-- DPS sort; the text is typed (a keystroke without its UpdateGem, R9); the box is drawn until
-- the DPS sort ends, then the list is built again for the typed text. `filter` is nil,
-- "support" or "grants_active_skill". Returns the ids in list order, PoB's DPS and colours
-- by id, and the base DPS.
function gemSearch(g, target, text, filter)
	local control = gemBox(g, target)
	control.sortGemsBy = filter
	control:OnFocusGained()
	control.buf = text
	control:BuildList(text)
	local draws = 0
	repeat
		control:Draw(viewport)
		draws = draws + 1
		assert(draws < 10000, "the DPS sort never ended")
	until not control.dpsBuilder and not control.dpsBuildFlag
	control:BuildList(text)
	local ids, dps, colours = {}, {}, {}
	for _, key in ipairs(control.list) do
		if key ~= "" then
			local id = key:gsub("^Default:", "")
			ids[#ids + 1] = id
			if control.sortCache.dpsColor[key] ~= "" then
				dps[id] = control.sortCache.dps[key]
				colours[id] = control.sortCache.dpsColor[key]
			end
		end
	end
	local base = control.sortCache.baseDPS
	control.dropped = false
	return { ids = ids, dps = dps, colours = colours, base_dps = base, draws = draws }
end

-- A gem tooltip as the box shows it (R15), recorded with the host's `glue.tooltip()` recorder.
-- With `id`, the hover over that row of the dropped list (GemSelectControl.lua:582-604);
-- without, the box's own gem (:629-646). The box's tooltip is swapped for the recorder.
function gemTooltip(g, target, id)
	local control = gemBox(g, target)
	local tab = skillsTab()
	local recorder = glue.tooltip()
	local saved = control.tooltip
	control.tooltip = recorder
	local ok, err = pcall(function()
		if id then
			local gemData = build.data.gems[id]
			assert(gemData, "no gem " .. id)
			local calcFunc, calcBase = tab.build.calcsTab:GetMiscCalculator(control.build)
			local output = control:CalcOutputWithThisGem(calcFunc, gemData, tab.sortGemsByDPSField == "FullDPS")
			local gemInstance = {
				level = tab:ProcessGemLevel(gemData, control.imbuedSelect),
				quality = tab.defaultGemQuality or 0,
				qualityId = "Default",
				count = 1,
				enabled = true,
				enableGlobal1 = true,
				enableGlobal2 = true,
				gemId = gemData.id,
				nameSpec = gemData.name,
				skillId = gemData.grantedEffectId,
				displayEffect = nil,
				gemData = gemData,
			}
			control:AddGemTooltip(gemInstance)
			control.tooltip:AddSeparator(10)
			tab.build:AddStatComparesToTooltip(control.tooltip, calcBase, output, "^7Selecting this gem will give you:")
		else
			local gemInstance = tab.displayGroup.gemList[control.index]
			if gemInstance and gemInstance.gemData then
				control:AddGemTooltip(gemInstance)
			end
		end
	end)
	control.tooltip = saved
	if not ok then error(err, 0) end
	return recorder:lines()
end
