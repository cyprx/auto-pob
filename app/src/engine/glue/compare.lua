-- The Compare tab (R4-R8): comparison builds, PoB's selectors, the Summary and the tree copy.
-- Runs after popups.lua (control views) and tree.lua.
--
-- Our frames always draw the Tree tab (builds.lua), so `CompareTab:Draw` never runs. What it
-- does before drawing, the glue does before every read and press (R6): rebuild a flagged
-- comparison, then `UpdateSetSelectors`, and fill the Items view's set lists as its draw does.

glue = glue or {}

local NOT_OFFERED = "PoB doesn't offer that here"
local FAILED = "Failed to import build for comparison."

-- R6's controls, in order, with the sub-view PoB shows each in (nil: the top bar, shown in
-- every sub-view once a comparison is loaded)
local CONTROLS = {
	{ "compareSpecSelect" },
	{ "compareSkillSetSelect" },
	{ "compareItemSetSelect" },
	{ "compareConfigSetSelect" },
	{ "cmpSocketGroup" },
	{ "cmpMainSkill" },
	{ "cmpSkillPart" },
	{ "cmpStageCount" },
	{ "cmpMineCount" },
	{ "cmpMinion" },
	{ "cmpMinionSkill" },
	{ "primaryItemSetSelect", "ITEMS" },
	{ "compareItemSetSelect2", "ITEMS" },
	{ "treeOverlayCheck", "TREE" },
	{ "itemsExpandedCheck", "ITEMS" },
}

local MODE_OF = {}
for _, entry in ipairs(CONTROLS) do
	MODE_OF[entry[1]] = entry[2] or false
end

local function compare_tab()
	if not glue.build_loaded() then
		error("open a build first", 0)
	end
	return pob.build.compareTab
end

-- The draw-only syncs (R6); returns the active comparison, or nil
local function sync(tab)
	local entry = tab:GetActiveCompare()
	if not entry then
		return nil
	end
	-- CompareTab.lua:1677-1680
	if entry.buildFlag then
		entry:Rebuild()
	end
	tab:UpdateSetSelectors(entry)
	-- the Items view's set lists (CompareTab.lua:1810-1818)
	local primaryItems = tab.primaryBuild.itemsTab
	if primaryItems and primaryItems.itemSetOrderList then
		tab:PopulateSetDropdown(primaryItems, "itemSetOrderList", "itemSets", "activeItemSetId", tab.controls.primaryItemSetSelect)
	end
	if entry.itemsTab and entry.itemsTab.itemSetOrderList then
		tab:PopulateSetDropdown(entry.itemsTab, "itemSetOrderList", "itemSets", "activeItemSetId", tab.controls.compareItemSetSelect2)
	end
	return entry
end

local function active_entry(tab)
	local entry = sync(tab)
	if not entry then
		error("no comparison build loaded", 0)
	end
	return entry
end

-- Runs `fn` with the Compare tab on sub-view `mode` (nil: as it is), as the user would be when
-- the control is in reach. Only `Draw` reads the mode, so it is set directly and put back.
local function in_mode(tab, mode, fn)
	local saved = tab.compareViewMode
	if mode then
		tab.compareViewMode = mode
	end
	local ok, err = pcall(fn)
	tab.compareViewMode = saved
	if not ok then
		error(err, 0)
	end
	return err
end

local function require_usable(control)
	if not (control:IsShown() and control:IsEnabled()) then
		error(NOT_OFFERED, 0)
	end
end

-- ---- Loading (R5) ----

-- PoB's ImportBuild, or its failure text. Nothing it opens stays open.
local function import(fn)
	local ok, imported
	glue.in_popups(function()
		ok, imported = pcall(fn)
	end)
	if not ok or not imported then
		error(FAILED, 0)
	end
end

-- A build file, as PoB's "Import from Folder" reads one (CompareTab.lua:1563-1579). The
-- comparison is named after the file, as PoB's list names it. Errors name the file only.
function glue.compare_load_file(path)
	local tab = compare_tab()
	local name = path:match("([^/\\]+)$") or path
	local label = name:gsub("%.[xX][mM][lL]$", "")
	local file = io.open(path, "r")
	if not file then
		error("Couldn't open '" .. name .. "'.", 0)
	end
	local xml = file:read("*a")
	file:close()
	if not xml or xml == "" then
		error("Build file is empty or unreadable.", 0)
	end
	import(function()
		return tab:ImportBuild(xml, label)
	end)
end

-- A share code (CompareTab.lua:1256)
function glue.compare_load_code(code)
	local tab = compare_tab()
	code = tostring(code):gsub("^%s+", ""):gsub("%s+$", "")
	import(function()
		return tab:ImportFromCode(code)
	end)
end

-- ---- Entries ----

local function require_index(tab, index)
	if #tab.compareEntries == 0 then
		error("no comparison build loaded", 0)
	end
	if index < 1 or index > #tab.compareEntries then
		error("no such comparison", 0)
	end
end

-- PoB's "Compare with:" dropdown
local function select_entry(tab, index)
	local drop = tab.controls.compareBuildSelect
	require_usable(drop)
	drop:SetSel(index)
end

function glue.compare_select(index)
	local tab = compare_tab()
	require_index(tab, index)
	select_entry(tab, index)
end

-- PoB's Remove button removes the active comparison: the entry is picked first
function glue.compare_remove(index)
	local tab = compare_tab()
	require_index(tab, index)
	select_entry(tab, index)
	local button = tab.controls.removeBtn
	require_usable(button)
	glue.in_popups(button.onClick)
end

-- ---- Controls (R6) ----

function glue.compare_set(name, kind, value, label)
	local tab = compare_tab()
	local mode = MODE_OF[name]
	if mode == nil then
		error("no such control", 0)
	end
	active_entry(tab)
	in_mode(tab, mode or nil, function()
		local control = tab.controls[name]
		require_usable(control)
		local _, control_kind = glue.describe_control(name, control)
		glue.in_popups(function()
			glue.set_control(control, control_kind, kind, value, label)
		end)
	end)
	-- a pick flags the comparison; PoB rebuilds it in its next draw
	sync(tab)
end

-- The Tree view's "Copy tree" and "Copy and use" buttons (R8)
function glue.compare_copy_tree(andUse)
	local tab = compare_tab()
	active_entry(tab)
	in_mode(tab, "TREE", function()
		local button = andUse and tab.controls.copySpecUseBtn or tab.controls.copySpecBtn
		require_usable(button)
		glue.in_popups(button.onClick)
	end)
end

-- Entering (`on`) or leaving the Compare tab's Tree view: PoB's sub-tab buttons when they are
-- shown (leaving the tree clears the overlay's spec, CompareTab.lua:209-213), the mode itself
-- while no comparison is loaded.
function glue.compare_overlay(on)
	local tab = compare_tab()
	local mode = on and "TREE" or "SUMMARY"
	if tab.compareViewMode == mode then
		return
	end
	local button = tab.controls[on and "subTabTree" or "subTabSummary"]
	if button:IsShown() then
		button.onClick()
	else
		tab.compareViewMode = mode
	end
end

-- ---- The view ----

-- What `DrawSummary` draws (R4): the headers, and the rows of `DrawStatList` grouped by their
-- `y`. The power report is not run (`RunComparePowerReport` is swapped out for the call).
local function summary(tab, entry)
	local strings, headers = {}, {}
	local in_list = false
	local draw_string = DrawString
	DrawString = function(x, y, align, height, font, text)
		if in_list then
			strings[#strings + 1] = { y = y, text = text }
		else
			headers[#headers + 1] = text
		end
	end
	local draw_stat_list = tab.DrawStatList
	tab.DrawStatList = function(self, ...)
		in_list = true
		local ok, drawY = pcall(draw_stat_list, self, ...)
		in_list = false
		if not ok then
			error(drawY, 0)
		end
		return drawY
	end
	tab.RunComparePowerReport = function() end
	local width, height = GetScreenSize()
	local ok, err = pcall(tab.DrawSummary, tab, { x = 0, y = 0, width = width, height = height }, entry)
	DrawString = draw_string
	tab.DrawStatList = nil
	tab.RunComparePowerReport = nil
	if not ok then
		error(err, 0)
	end
	local rows, row, y = {}, nil, nil
	for _, s in ipairs(strings) do
		if not row or s.y ~= y then
			row = {}
			rows[#rows + 1] = row
			y = s.y
		end
		row[#row + 1] = s.text
	end
	local positive, negative = colorCodes.POSITIVE, colorCodes.NEGATIVE
	local out = {}
	for i, texts in ipairs(rows) do
		local diff = texts[4] or ""
		local better
		if diff:sub(1, #positive) == positive then
			better = true
		elseif diff:sub(1, #negative) == negative then
			better = false
		end
		out[i] = { label = texts[1] or "", ours = texts[2] or "", theirs = texts[3] or "", diff = diff, better = better }
	end
	-- "Stat", this build, the comparison, "Difference" (CompareTab.lua:3175-3179)
	return out, { headers[2] or "", headers[3] or "" }
end

function glue.compare_view()
	local tab = pob.build.compareTab
	if not tab or #tab.compareEntries == 0 then
		return nil
	end
	local entry = active_entry(tab)
	local entries = {}
	for i, compareEntry in ipairs(tab.compareEntries) do
		entries[i] = compareEntry.label or ("Build " .. i)
	end
	local controls = {}
	for _, spec in ipairs(CONTROLS) do
		local name, mode = spec[1], spec[2]
		in_mode(tab, mode, function()
			controls[#controls + 1] = (glue.describe_control(name, tab.controls[name]))
		end)
	end
	local rows, headers = summary(tab, entry)
	return {
		entries = entries,
		active = tab.activeCompareIndex - 1,
		headers = headers,
		controls = controls,
		summary = rows,
	}
end
