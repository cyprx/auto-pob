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
		specs[i] = {
			title = spec.title or "Default", version = spec.treeVersion, alloc = alloc,
			class_id = spec.curClassId, ascend_id = spec.curAscendClassId,
		}
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

-- ---- Calcs (R24) ----

-- Each section after PoB's own UpdateSize (CalcSectionControl.lua:103-150): the width it gave
-- each sub-section's cells (0 where no row of the sub-section is shown), and whether its "^"
-- shows
function upCalcsWidths()
	local out = {}
	local calcs = build.calcsTab
	for _, section in ipairs(calcs.sectionList) do
		section:UpdateSize()
		local widths = {}
		for i, sub in ipairs(section.subSection) do
			widths[i] = 0
			for _, row in ipairs(sub.data) do
				if calcs:CheckFlag(row) and row[1] and row[1].width then
					widths[i] = row[1].width
					break
				end
			end
		end
		out[#out + 1] = {
			id = section.id,
			widths = widths,
			pop_out = section.controls.popOut:IsShown() and true or false,
		}
	end
	return out
end

-- ---- Tree: heat map Custom depth, power report filter (NF-5, NF-6), Version (NF-7) ----

-- The depth dropdown's "Custom" and its box typed into, then the stat dropdown's pick, and
-- PoB's own power builder run to its end (its callback hands the report to the list)
function upPowerCustom(statLabel, depthText)
	local tab = build.treeTab
	local depth = tab.controls.nodePowerMaxDepthSelect
	for i, entry in ipairs(depth.list) do
		if entry == "Custom" then depth:SetSel(i) end
	end
	assert(tab.controls.nodePowerMaxDepthCustom:IsShown(), "no Custom box")
	tab.controls.nodePowerMaxDepthCustom:SetText(depthText, true)
	local stat
	for _, entry in ipairs(tab.powerStatList) do
		if entry.label == statLabel then stat = entry end
	end
	tab.controls.treeHeatMapStatSelect.selFunc(nil, stat)
	runCallback("OnFrame")
	repeat build.calcsTab:BuildPower() until not build.calcsTab.powerBuilder
	return build.calcsTab.nodePowerMaxDepth
end

-- The rows PoB's report list shows, as "id name", sorted (the report is built with `pairs`)
function upReportListed()
	local out = {}
	for _, row in ipairs(build.treeTab.controls.powerReportList.list) do
		out[#out + 1] = row.id .. " " .. row.name
	end
	table.sort(out)
	return out
end

function upReportFilter(index)
	build.treeTab.controls.powerReportList.controls.filterSelect:SetSel(index + 1)
end

function upReportMasteries(on)
	local check = build.treeTab.controls.powerReportList.controls.masteryCheck
	check.state = on
	check.changeFunc(on)
end

-- The Version dropdown's entries
function upVersions()
	local out = {}
	for i, entry in ipairs(build.treeTab.controls.versionSelect.list) do
		out[i] = { label = entry.label, value = entry.value }
	end
	return out
end

-- The Version dropdown's pick and its popup's button
function upConvertVersion(version, button)
	local select = build.treeTab.controls.versionSelect
	for i, entry in ipairs(select.list) do
		if entry.value == version then select:SetSel(i) end
	end
	pressLabel(button)
	runCallback("OnFrame")
end

-- ---- Import (NF-8) and the notes' zoom ----

-- The account box typed into: whether PoB shows "Missing discriminator" and enables Start
function upAccountName(text)
	local controls = build.importTab.controls
	controls.siteAccountName:SetText(text)
	return {
		controls.siteAccountNameMissingDiscriminator:IsShown() and true or false,
		controls.siteAccountNameGo:IsEnabled() and true or false,
	}
end

-- A successful list's SaveAccountHistory (ImportTab.lua:800-811), for a name in the box; used
-- on both sides to fill the history the same way
function saveAccountHistory(b, name)
	b = b or build
	b.importTab.controls.siteAccountName:SetText(name)
	b.importTab:SaveAccountHistory()
end

function accountHistory(b)
	b = b or build
	local out = {}
	for i, name in ipairs(b.importTab.controls.siteAccountHistory.list) do out[i] = name end
	return out
end

-- The history dropdown's pick and its X
function upRemoveAccount(name)
	local controls = build.importTab.controls
	local history = controls.siteAccountHistory
	for i, entry in ipairs(history.list) do
		if entry == name then history:SetSel(i) end
	end
	controls.siteRemoveAccount.onClick()
end

-- Ctrl and a key on the notes box, as EditControl:OnKeyDown passes it to ZoomText
function upNotesZoom(keys)
	local edit = build.notesTab.controls.edit
	local heights = {}
	for i = 1, #keys do
		edit:ZoomText(keys:sub(i, i))
		heights[i] = edit.lineHeight
	end
	return heights
end

-- The "Convert to <latest>" button, and the message it shows, read then dismissed with Ok
function upConvertLatest()
	build.treeTab.controls.specConvert.onClick()
	local popup = main.popups[1]
	local lines = {}
	for _, control in ipairs(popup.controls) do lines[#lines + 1] = control:GetProperty("label") end
	local message = { title = popup:GetProperty("title"), text = table.concat(lines, "\n") }
	pressLabel("Ok")
	runCallback("OnFrame")
	return message
end

-- ---- The canvas's Class Change (PassiveTreeView.lua:474) ----

-- The lowest id of a node of the named ascendancy that isn't its start
function ascendancyNode(name, b)
	b = b or build
	local best
	for id, node in pairs(b.spec.nodes) do
		if node.ascendancyName == name and node.type ~= "AscendClassStart" and (not best or id < best) then
			best = id
		end
	end
	return best
end

-- A left click on the node, through the tree view's own Draw: the cursor is put over the node
-- (the view's own transform, PassiveTreeView.lua:265-276) for a press and release, then the
-- question's `button` is pressed. Returns the question's title, nil when PoB asked nothing.
function upClickNode(nodeId, button)
	local viewer = build.treeTab.viewer
	local node = build.spec.nodes[nodeId]
	local viewPort = { x = 0, y = 0, width = 2000, height = 2000 }
	viewer.zoom, viewer.zoomX, viewer.zoomY = 1, 0, 0
	local scale = math.min(viewPort.width, viewPort.height) / build.spec.tree.size * viewer.zoom
	local x = node.x * scale + viewer.zoomX + viewPort.width / 2
	local y = node.y * scale + viewer.zoomY + viewPort.height / 2
	local cursor = GetCursorPos
	GetCursorPos = function() return x, y end
	local depth = #main.popups
	local ok, err = pcall(viewer.Draw, viewer, build, viewPort, {
		{ type = "KeyDown", key = "LEFTBUTTON" },
		{ type = "KeyUp", key = "LEFTBUTTON" },
	})
	GetCursorPos = cursor
	assert(ok, err)
	local title
	if #main.popups > depth then
		title = main.popups[1]:GetProperty("title")
		pressLabel(button)
	end
	runCallback("OnFrame")
	return title
end
