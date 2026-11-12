-- Task 13a's upstream side: the Skills, Config, Calcs and Tree tabs' parity gaps, by pressing
-- PoB's own controls (list drops through ListControl:OnKeyUp, tooltips through the control's
-- own DrawTooltip). Loaded into upstream's state, and into ours for the shared readers
-- (`tabState`), which take the build as an argument.

local function upBuild(b)
	return b or build
end

-- The sets, trees and flags the acts change, of `b` (the wrapper's `build` by default)
function tabState(b)
	b = upBuild(b)
	local skillSets, configSets, specs = {}, {}, {}
	for i, id in ipairs(b.skillsTab.skillSetOrderList) do
		skillSets[i] = { id, b.skillsTab.skillSets[id].title or "Default" }
	end
	for i, id in ipairs(b.configTab.configSetOrderList) do
		configSets[i] = { id, b.configTab.configSets[id].title or "Default" }
	end
	for i, spec in ipairs(b.treeTab.specList) do
		local alloc = {}
		for id in pairs(spec.allocNodes) do alloc[#alloc + 1] = id end
		table.sort(alloc)
		specs[i] = { title = spec.title or "Default", version = spec.treeVersion, alloc = alloc }
	end
	local itemSpecs = {}
	for i, title in ipairs(b.itemsTab.controls.specSelect.list) do itemSpecs[i] = title end
	return {
		skill_sets = skillSets,
		active_skill_set = b.skillsTab.activeSkillSetId,
		config_sets = configSets,
		active_config_set = b.configTab.activeConfigSetId,
		specs = specs,
		active_spec = b.treeTab.activeSpec,
		items_spec_list = itemSpecs,
		items_spec_sel = b.itemsTab.controls.specSelect.selIndex,
		flags = {
			build = b.modFlag and true or false,
			spec = b.spec.modFlag and true or false,
			tree = b.treeTab.modFlag and true or false,
			skills = b.skillsTab.modFlag and true or false,
			items = b.itemsTab.modFlag and true or false,
			config = b.configTab.modFlag and true or false,
			calcs = b.calcsTab.modFlag and true or false,
			build_flag = b.buildFlag and true or false,
		},
		undo = {
			spec = #b.spec.undo,
			skills = #b.skillsTab.undo,
			items = #b.itemsTab.undo,
			config = #b.configTab.undo,
			calcs = #b.calcsTab.undo,
		},
	}
end

-- A PoB tooltip's lines, in `TooltipLine`'s shape
local function lines(tooltip)
	local out = {}
	for i, line in ipairs(tooltip.lines) do
		if line.text then
			out[i] = { size = line.size, text = line.text }
		else
			out[i] = { separator = line.size }
		end
	end
	return out
end

-- The newest popup's list control (a set manager's), as the user sees it
local function popupList()
	for _, control in pairs(main.popups[1].controls) do
		if control.controls and control.controls.new and control.OnSelDelete then
			return control
		end
	end
	error("no list in the popup")
end

-- Presses the newest popup's button labelled `label`
local function pressLabel(label)
	for _, control in pairs(main.popups[1].controls) do
		if control.label == label and control.onClick then
			control.onClick()
			return
		end
	end
	error("no " .. label .. " button")
end

-- A row of a list dragged and dropped (ListControl.lua:407-436): the press picks the row, the
-- drag sets the target (`selDragIndex` is the row it lands before), the release drops it.
-- `from` and `to` are 1-based, `to` the row's final place.
local function dragRow(list, from, to)
	list.selIndex, list.selValue = from, list.list[from]
	list.selDragActive = true
	list.selDragIndex = to > from and to + 1 or to
	list:OnKeyUp("LEFTBUTTON")
end

-- ---- Skills (R31, R22) ----

-- The skill set manager's list: drag a row and press Done
function upMoveSkillSet(from, to)
	build.skillsTab:OpenSkillSetManagePopup()
	dragRow(popupList(), from + 1, to + 1)
	pressLabel("Done")
	runCallback("OnFrame")
end

-- A click on the group's row, then the gem box's own hover draw (SkillsTab.lua:861-1000)
function upGemHover(group, slot, which)
	local tab = build.skillsTab
	local list = tab.controls.groupList
	local value = tab.socketGroupList[group + 1]
	list.selIndex, list.selValue = group + 1, value
	list:OnSelect(group + 1, value)
	local control = tab.gemSlots[slot + 1][which]
	assert(control:IsShown() and control:IsEnabled(), "the box isn't offered")
	control:DrawTooltip(0, 0, 10, 10, main.viewPort)
	return lines(control.tooltip)
end

-- ---- Config (R22, NF-4) ----

-- An option's control drawn hovered, with what its own Draw passes (CheckBoxControl.lua:80,
-- DropDownControl.lua:295-298, EditControl.lua:285)
function upConfigHover(var)
	local control = build.configTab.varControls[var]
	assert(control and control:IsShown(), "the option isn't shown")
	if control._className == "CheckBoxControl" then
		control:DrawTooltip(0, 0, 10, 10, main.viewPort, control.state)
	elseif control._className == "DropDownControl" then
		control:DrawTooltip(0, 0, 10, 10, main.viewPort, "BODY", control.selIndex, control.list[control.selIndex])
	else
		control:DrawTooltip(0, 0, 10, 10, main.viewPort)
	end
	return lines(control.tooltip)
end

-- A custom modifier group's text typed in, and its enable box drawn hovered
function upCustomModText(group, text)
	build.configTab.customModsBlockControls[group + 1].controls.textEdit:SetText(text, true)
	runCallback("OnFrame")
end

function upCustomModHover(group)
	local check = build.configTab.customModsBlockControls[group + 1].controls.enableCheck
	check:DrawTooltip(0, 0, 10, 10, main.viewPort, check.state)
	return lines(check.tooltip)
end

-- Types `title` into the open name popup and presses Save
local function saveName(title)
	local popup = main.popups[1]
	popup.controls.edit:SetText(title, true)
	assert(popup.controls.save:IsEnabled(), "Save is disabled")
	popup.controls.save.onClick()
end

-- The config set manager: a click on the row, Copy, the name popup, then Done
function upCopyConfigSet(index, title)
	build.configTab:OpenConfigSetManagePopup()
	local list = popupList()
	list.selIndex, list.selValue = index + 1, list.list[index + 1]
	list.controls.copy.onClick()
	saveName(title)
	pressLabel("Done")
	runCallback("OnFrame")
end

function upMoveConfigSet(from, to)
	build.configTab:OpenConfigSetManagePopup()
	dragRow(popupList(), from + 1, to + 1)
	pressLabel("Done")
	runCallback("OnFrame")
end

-- ---- Trees (NF-3) ----

function upCopySpec(index, title)
	build.treeTab:OpenSpecManagePopup()
	local list = popupList()
	list.selIndex, list.selValue = index + 1, list.list[index + 1]
	list.controls.copy.onClick()
	saveName(title)
	pressLabel("Done")
	runCallback("OnFrame")
end

function upMoveSpec(from, to)
	build.treeTab:OpenSpecManagePopup()
	dragRow(popupList(), from + 1, to + 1)
	pressLabel("Done")
	runCallback("OnFrame")
end

-- The tree dropdown's pick (TreeTab.lua:39-43)
function upSelectSpec(index)
	local select = build.treeTab.controls.specSelect
	select.selFunc(index + 1, select.list[index + 1])
	runCallback("OnFrame")
end
