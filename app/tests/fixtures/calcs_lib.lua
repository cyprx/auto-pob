-- Helpers for the Calcs recipes. Runs in unmodified upstream PoB after a build is loaded.
-- Written independently of glue/calcs.lua: it walks upstream's own `sectionList` and applies
-- the rules of CalcSectionControl:UpdateSize and :DrawContent. Nothing here may call our glue.

local function actorOf(calcs)
	return calcs.input.showMinion and calcs.calcsEnv.minion or calcs.calcsEnv.player
end

local function hasControl(rowData)
	for _, colData in ipairs(rowData) do
		if colData.control then return true end
	end
end

-- The sections as plain tables, as the Calcs tab would show them.
function calcsDump()
	local calcs = build.calcsTab
	local actor = actorOf(calcs)
	local out = {}
	for _, section in ipairs(calcs.sectionList) do
		local anyRow = false
		if calcs:CheckFlag(section) then
			for _, subSec in ipairs(section.subSection) do
				for _, rowData in ipairs(subSec.data) do
					anyRow = anyRow or calcs:CheckFlag(rowData) and true
				end
			end
		end
		local subs = {}
		for subIndex, subSec in ipairs(section.subSection) do
			local sub = {
				label = subSec.label,
				collapsed = not not subSec.collapsed,
				search_match = false,
				rows = {},
			}
			if anyRow then
				sub.search_match = not not calcs:SearchMatch(subSec.label)
				if subSec.data.extra then
					sub.extra = tostring(formatCalcStr(subSec.data.extra, actor))
				end
				if not subSec.collapsed then
					for rowIndex, rowData in ipairs(subSec.data) do
						if calcs:CheckFlag(rowData) and not hasControl(rowData) then
							local row = {
								index = rowIndex - 1,
								label = rowData.label,
								colour = rowData.color,
								text_size = rowData.textSize or 14,
								search_match = not not (rowData.label and calcs:SearchMatch(rowData.label)),
								cells = {},
							}
							for colIndex, colData in ipairs(rowData) do
								if colData.format and calcs:CheckFlag(colData) then
									table.insert(row.cells, {
										index = colIndex - 1,
										text = tostring(formatCalcStr(colData.format, actor, colData)),
										breakdown = #colData > 0,
									})
								end
							end
							table.insert(sub.rows, row)
						end
					end
				end
			end
			table.insert(subs, sub)
			-- DrawContent (CalcSectionControl.lua:468-472) returns here for the first sub-section
			if subIndex == 1 and (not anyRow or subSec.collapsed) then break end
		end
		table.insert(out, {
			id = section.id,
			group = section.group,
			width_cols = section.widthCols,
			colour = section.colour,
			enabled = not not anyRow,
			subs = subs,
		})
	end
	return out
end

-- Picks a calculation mode in the SkillSelect section's dropdown and runs a frame, so
-- upstream recalculates.
function setMode(buffMode)
	for _, section in ipairs(build.calcsTab.sectionList) do
		if section.id == "SkillSelect" then
			for _, subSec in ipairs(section.subSection) do
				for _, rowData in ipairs(subSec.data) do
					for _, colData in ipairs(rowData) do
						if colData.controlName == "mode" then
							-- the section's updateFunc does this in a frame (CalcsTab.lua:141)
							colData.control:SelByValue(build.calcsTab.input.misc_buffMode, "buffMode")
							for i, item in ipairs(colData.control.list) do
								if item.buffMode == buffMode then
									colData.control:SetSel(i)
								end
							end
						end
					end
				end
			end
		end
	end
	runCallback("OnFrame")
end

-- Selector helpers. A frame would run the SkillSelect section's own `updateFunc` (it syncs
-- the dropdowns with `calcsTab.input` and the build, CalcsTab.lua:138); run it before
-- reading or pressing a control.
local function skillSelect()
	for _, section in ipairs(build.calcsTab.sectionList) do
		if section.id == "SkillSelect" then return section end
	end
end

local function rowOf(section, controlName)
	for _, subSec in ipairs(section.subSection) do
		for _, rowData in ipairs(subSec.data) do
			for _, colData in ipairs(rowData) do
				if colData.controlName == controlName then return rowData end
			end
		end
	end
end

local function refreshedSelect()
	local section = skillSelect()
	section:updateFunc()
	return section
end

-- Presses a dropdown by PoB's control name (`select` would hide Lua's own); 1-based.
function calcsSelect(controlName, index)
	refreshedSelect().controls[controlName]:SetSel(index)
	runCallback("OnFrame")
end

-- Types into the stage or mine count: the edit box filters the text itself.
function calcsSetCount(controlName, text)
	local control = refreshedSelect().controls[controlName]
	control:SetText("")
	control:Insert(text)
	runCallback("OnFrame")
end

function calcsShowMinion(show)
	local control = refreshedSelect().controls.showMinion
	control.state = show
	control.changeFunc(show)
	runCallback("OnFrame")
end

function calcsCountBuf(controlName)
	return skillSelect().controls[controlName].buf
end

-- The selector as the Calcs tab would show it.
function selectorDump()
	local calcs = build.calcsTab
	local section = refreshedSelect()
	local function visible(name)
		local control = section.controls[name]
		return control and control.shown ~= false and calcs:CheckFlag(rowOf(section, name)) and control
	end
	local function dropList(name)
		local control = visible(name)
		if not control then return nil end
		local labels = {}
		for i, item in ipairs(control.list) do
			labels[i] = type(item) == "table" and item.label or item
		end
		return { labels = labels, selected = control.selIndex - 1, enabled = control.enabled ~= false }
	end
	local stages, mines = visible("mainSkillStageCount"), visible("mainSkillMineCount")
	local showMinion = visible("showMinion")
	local minionShown = nil
	if showMinion then minionShown = showMinion.state and true or false end
	return {
		group = dropList("mainSocketGroup"),
		skill = dropList("mainSkill"),
		part = dropList("mainSkillPart"),
		stages = stages and stages.buf or nil,
		mines = mines and mines.buf or nil,
		show_minion = minionShown,
		minion = dropList("mainSkillMinion"),
		minion_skill = dropList("mainSkillMinionSkill"),
		mode = dropList("mode"),
		mode_tooltip = section.controls.mode.tooltipText,
	}
end
