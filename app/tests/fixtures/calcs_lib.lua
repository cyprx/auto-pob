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

-- The first cell that shows, has a breakdown and whose format reads `output.<statKey>`, as a
-- 0-based CellRef; nil when there is none.
function findCell(statKey)
	local calcs = build.calcsTab
	local needle = "output:" .. statKey .. "}"
	for _, section in ipairs(calcs.sectionList) do
		local anyRow = false
		if calcs:CheckFlag(section) then
			for _, subSec in ipairs(section.subSection) do
				for _, rowData in ipairs(subSec.data) do
					anyRow = anyRow or calcs:CheckFlag(rowData) and true
				end
			end
		end
		if anyRow then
			for subIndex, subSec in ipairs(section.subSection) do
				if not subSec.collapsed then
					for rowIndex, rowData in ipairs(subSec.data) do
						if calcs:CheckFlag(rowData) and not hasControl(rowData) then
							for colIndex, colData in ipairs(rowData) do
								if colData.format and calcs:CheckFlag(colData) and #colData > 0
									and colData.format:find(needle, 1, true) then
									return { section = section.id, sub = subIndex - 1,
										row = rowIndex - 1, cell = colIndex - 1 }
								end
							end
						end
					end
				end
			end
		end
	end
end

-- What upstream's breakdown control would show for a cell, as plain tables.
function breakdownDump(ref)
	local calcs = build.calcsTab
	local out = { cell = ref, sections = {} }
	local colData
	for _, section in ipairs(calcs.sectionList) do
		if section.id == ref.section then
			local subSec = section.subSection[ref.sub + 1]
			local rowData = subSec and subSec.data[ref.row + 1]
			colData = rowData and rowData[ref.cell + 1]
		end
	end
	if not colData or not calcs:CheckFlag(colData) then return out end
	local control = calcs.controls.breakdown
	-- the same data again would return early
	control:SetBreakdownData()
	control:SetBreakdownData(colData, false)
	if not control.shown then return out end
	for _, section in ipairs(control.sectionList) do
		if section.type == "TEXT" then
			local lines = {}
			for i, line in ipairs(section.lines) do
				lines[i] = formatNumSep(line)
			end
			table.insert(out.sections, { kind = "text", lines = lines, size = section.textSize })
		elseif section.type == "RADIUS" then
			table.insert(out.sections, { kind = "radius", radius = section.radius })
		elseif section.type == "TABLE" then
			local columns, rows = {}, {}
			for _, col in ipairs(section.colList) do
				-- upstream gives a column a width when some row has a value for it
				if col.width then
					table.insert(columns, { label = col.label, key = col.key, right = not not col.right })
				end
			end
			for _, rowItem in ipairs(section.rowList) do
				local row = { cells = {}, tooltips = {}, nodes = {} }
				for _, col in ipairs(columns) do
					local value = rowItem[col.key]
					if value then
						local _, alpha = string.gsub(value, "%a", " ")
						local _, notes = string.gsub(value, " to ", " ")
						local _, paren = string.gsub(value, "%b()", " ")
						if alpha == 0 or notes > 0 or paren > 0 then
							row.cells[col.key] = formatNumSep(tostring(value))
						else
							row.cells[col.key] = tostring(value)
						end
						local tipFunc = rowItem[col.key .. "Tooltip"]
						if tipFunc then
							local tip = new("Tooltip")
							tip:Clear()
							tipFunc(tip)
							local lines = {}
							for _, line in ipairs(tip.lines) do
								if line.text then
									table.insert(lines, { size = line.size, text = line.text })
								else
									-- a separator has no text
									table.insert(lines, { separator = line.size })
								end
							end
							if #lines > 0 then row.tooltips[col.key] = lines end
						end
						local node = rowItem[col.key .. "Node"]
						if node and node.x then row.nodes[col.key] = node.id end
					end
				end
				table.insert(rows, row)
			end
			table.insert(out.sections, { kind = "table", label = section.label,
				footer = section.footer, columns = #columns > 0 and columns or nil,
				rows = #rows > 0 and rows or nil })
		end
	end
	return out
end
