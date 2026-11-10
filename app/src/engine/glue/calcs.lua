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

local selector_view

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
	view.selector = selector_view(calcs)
	local actor = calcs.input.showMinion and env.minion or env.player
	for _, section in ipairs(calcs.sectionList) do
		view.sections[#view.sections + 1] = section_view(calcs, section, actor)
	end
	return view
end

-- The SkillSelect section: PoB syncs its dropdowns with `calcsTab.input` and the build in the
-- section's `updateFunc`, which only runs while the section draws. Run it by hand before
-- reading or pressing a control, so `selIndex` matches the model.
local function skill_select(calcs)
	for _, section in ipairs(calcs.sectionList) do
		if section.id == "SkillSelect" then return section end
	end
	error("PoB's Calcs tab has no skill selector", 0)
end

local function refreshed_select(calcs)
	local section = skill_select(calcs)
	section:updateFunc()
	return section
end

local function row_of(section, name)
	for _, sub in ipairs(section.subSection) do
		for _, row in ipairs(sub.data) do
			for _, colData in ipairs(row) do
				if colData.controlName == name then return row end
			end
		end
	end
end

-- The control, when the selector shows it: `RefreshSkillSelectControls` left it shown (the
-- socket group, mode and minion check box are never hidden) and its row's flags hold.
local function shown_control(calcs, section, name, always_shown)
	local control = section.controls[name]
	if not control then return nil end
	if not always_shown and control.shown ~= true then return nil end
	if not calcs:CheckFlag(row_of(section, name)) then return nil end
	return control
end

local function drop_list(control)
	local labels = {}
	for i, item in ipairs(control.list) do
		labels[i] = type(item) == "table" and item.label or item
	end
	return {
		labels = labels,
		selected = math.max((control.selIndex or 1) - 1, 0),
		enabled = control.enabled ~= false,
	}
end

function selector_view(calcs)
	local section = refreshed_select(calcs)
	local function drop(name, always_shown)
		local control = shown_control(calcs, section, name, always_shown)
		return control and drop_list(control) or nil
	end
	local function text(name)
		local control = shown_control(calcs, section, name)
		return control and (control.buf or "") or nil
	end
	local minion_check = shown_control(calcs, section, "showMinion", true)
	local show_minion = nil
	if minion_check then show_minion = minion_check.state and true or false end
	return {
		group = drop("mainSocketGroup", true),
		skill = drop("mainSkill"),
		part = drop("mainSkillPart"),
		stages = text("mainSkillStageCount"),
		mines = text("mainSkillMineCount"),
		show_minion = show_minion,
		minion = drop("mainSkillMinion"),
		minion_skill = drop("mainSkillMinionSkill"),
		mode = drop("mode", true),
		mode_tooltip = section.controls.mode.tooltipText or "",
	}
end

-- A control of the selector, after the model is synced. Hidden ones can be pressed too: the
-- model decides what is shown, and the UI only offers what is.
local function selector_control(name)
	local control = refreshed_select(pob.build.calcsTab).controls[name]
	if not control then error("no such control", 0) end
	return control
end

-- `index` is 1-based. PoB's own `selFunc` runs; `SetSel` clamps, so check the range first.
function glue.calcs_select(name, index)
	local control = selector_control(name)
	if index < 1 or index > #control.list then error("no such choice", 0) end
	control:SetSel(index)
end

-- The edit boxes filter their text with the pattern in `filter` ("%D": digits only)
function glue.calcs_set_count(name, text)
	local control = selector_control(name)
	control.buf = text:gsub("%D", "")
	control.changeFunc(control.buf)
end

function glue.calcs_show_minion(show)
	local control = selector_control("showMinion")
	control.state = show
	control.changeFunc(show)
	glue.frame()
end

function glue.calcs_search(text)
	pob.build.calcsTab.controls.search.buf = text
end

function glue.calcs_collapse(id, sub, collapsed)
	local calcs = pob.build.calcsTab
	for _, section in ipairs(calcs.sectionList) do
		if section.id == id then
			local subSec = section.subSection[sub]
			if not subSec then error("no such sub-section", 0) end
			subSec.collapsed = collapsed
			calcs.modFlag = true
			glue.frame()
			return
		end
	end
	error("no such section", 0)
end

function glue.calcs_undo()
	local calcs = pob.build.calcsTab
	calcs:Undo()
	calcs.build.buildFlag = true
end

function glue.calcs_redo()
	local calcs = pob.build.calcsTab
	calcs:Redo()
	calcs.build.buildFlag = true
end
