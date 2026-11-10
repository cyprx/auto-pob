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
			if anyRow and subIndex == 1 and subSec.collapsed then break end
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
