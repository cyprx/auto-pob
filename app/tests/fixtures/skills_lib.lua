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
		out.sets[i] = { id = id, title = tab.skillSets[id].title or "Default" }
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
	for i, choice in ipairs(c.groupSlot.list) do
		if choice.slotName == name then
			c.groupSlot:SetSel(i)
		end
	end
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
	local dump = skillsDump()
	table.insert(actions, { check = {
		sets = dump.sets, active_set = dump.active_set, groups = dump.groups,
		can_undo = dump.can_undo, can_redo = dump.can_redo, sidebar = sidebar_lines(),
	} })
end
