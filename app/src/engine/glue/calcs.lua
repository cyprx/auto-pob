-- The Calcs tab's sections, as `CalcSectionControl:DrawContent` would show them.
-- PoB's Draw functions don't run here, so `section.enabled` and `rowData.enabled` (set by
-- `UpdateSize`) can't be trusted: the flags are worked out with `calcsTab:CheckFlag`.

glue = glue or {}

-- PoB's `calcsTab:SearchMatch`, without the truthy-number result
local function search_match(calcs, text)
	return calcs:SearchMatch(text) and true or false
end

local function cells_of(calcs, row, actor)
	local cells = {}
	for index, colData in ipairs(row) do
		if colData.format and calcs:CheckFlag(colData) then
			cells[#cells + 1] = {
				index = index - 1,
				text = (formatCalcStr(colData.format, actor, colData)),
				breakdown = #colData > 0,
			}
		end
	end
	return cells
end

local function has_control(row)
	for _, colData in ipairs(row) do
		if colData.control then return true end
	end
	return false
end

local function section_view(calcs, section, actor)
	-- UpdateSize: the section shows if its own flags hold and some row does
	local enabled = false
	if calcs:CheckFlag(section) then
		for _, sub in ipairs(section.subSection) do
			for _, row in ipairs(sub.data) do
				if calcs:CheckFlag(row) then enabled = true end
			end
		end
	end
	local subs = {}
	for i, sub in ipairs(section.subSection) do
		local collapsed = sub.collapsed and true or false
		local view = {
			label = sub.label or "",
			collapsed = collapsed,
			search_match = false,
			rows = {},
		}
		if enabled then
			view.search_match = search_match(calcs, sub.label or "")
			if sub.data.extra then
				view.extra = (formatCalcStr(sub.data.extra, actor))
			end
			if not collapsed then
				for index, row in ipairs(sub.data) do
					-- the selector's controls are Task 2's
					if calcs:CheckFlag(row) and not has_control(row) then
						view.rows[#view.rows + 1] = {
							index = index - 1,
							label = row.label,
							colour = row.color,
							text_size = row.textSize or 14,
							search_match = row.label ~= nil and search_match(calcs, row.label),
							cells = cells_of(calcs, row, actor),
						}
					end
				end
			end
		end
		subs[#subs + 1] = view
		-- DrawContent returns after the first sub-section when the section is disabled or
		-- that sub-section is collapsed
		if i == 1 and (not enabled or collapsed) then break end
	end
	return {
		id = section.id,
		group = section.group,
		width_cols = section.widthCols,
		colour = section.colour,
		enabled = enabled,
		subs = subs,
	}
end

function glue.calcs_view()
	local calcs = pob.build.calcsTab
	local view = {
		sections = {},
		search = calcs.controls.search.buf or "",
		can_undo = calcs.undo[2] ~= nil,
		can_redo = calcs.redo[1] ~= nil,
	}
	local env = calcs.calcsEnv
	if not env then return view end
	local actor = calcs.input.showMinion and env.minion or env.player
	for _, section in ipairs(calcs.sectionList) do
		view.sections[#view.sections + 1] = section_view(calcs, section, actor)
	end
	return view
end
