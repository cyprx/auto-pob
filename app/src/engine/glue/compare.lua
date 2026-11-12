-- The Compare tab (R4-R8): comparison builds, PoB's selectors, the Summary and the tree copy.
-- Runs after popups.lua (control views) and tree.lua.
--
-- Our frames always draw the Tree tab (builds.lua), so `CompareTab:Draw` never runs. What it
-- does before drawing, the glue does before every read and press (R6): rebuild a flagged
-- comparison, then `UpdateSetSelectors`, and fill the Items view's set lists as its draw does.

glue = glue or {}

local NOT_OFFERED = "PoB doesn't offer that here"
-- PoB's Items view helpers: CompareTab keeps its own instance (CompareTab.lua:12)
local trade_helpers = LoadModule("Classes/TradeHelpers")
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
	-- the Tree view's version dropdowns, which open PoB's convert popup (`compare_convert_tree`)
	{ "leftVersionSelect", "TREE" },
	{ "rightVersionSelect", "TREE" },
	-- "Show All Configurations": a button PoB labels by its state; set as a check
	{ "configToggleBtn", "CONFIG" },
}

-- PoB's config option filter (CompareTab.lua:17)
local config_visibility = LoadModule("Modules/ConfigVisibility")

local MODE_OF = {}
for _, entry in ipairs(CONTROLS) do
	MODE_OF[entry[1]] = entry[2] or false
end

-- An Import-tab comparison landed: the tree's overlay may have changed (taken by the engine's
-- network step), and PoB switched to the Compare tab (taken by the next view), R5
-- Both belong to the Compare tab they were set for, so a build change drops them.
local landed, switched = nil, nil

-- `at_once`: a code, answered by its own command (which flags the tree itself)
function glue.compare_landed(at_once)
	local tab = pob.build.compareTab
	switched = tab
	if not at_once then
		landed = tab
	end
end

function glue.compare_take_landed()
	local was = landed ~= nil and landed == (glue.build_loaded() and pob.build.compareTab)
	landed = nil
	return was
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
	-- the Tree view's version dropdowns follow the specs (CompareTab.lua:1973-1989)
	for _, side in ipairs({ { tab.primaryBuild.spec, "leftVersionSelect" }, { entry.spec, "rightVersionSelect" } }) do
		if side[1] then
			for i, ver in ipairs(tab.treeVersionDropdownList) do
				if ver.value == side[1].treeVersion then
					tab.controls[side[2]].selIndex = i
					break
				end
			end
		end
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
		-- PoB names the full path; errors here name the file only (no paths in errors)
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
		if name == "configToggleBtn" then
			-- pressed only when PoB's state differs from the one asked for
			if kind ~= "check" then
				error("no such control", 0)
			end
			if (tab.configToggle and true or false) ~= value then
				glue.in_popups(control.onClick)
			end
			return
		end
		if name == "leftVersionSelect" or name == "rightVersionSelect" then
			-- a pick opens PoB's convert popup: `compare_convert_tree` answers it
			error(NOT_OFFERED, 0)
		end
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

-- PoB's version dropdown on the Tree view, this build's (`leftVersionSelect`) or the
-- comparison's (`rightVersionSelect`): the pick opens PoB's "Convert to Version" popup
-- (TreeTab.lua:649-666), whose "Convert" or "Copy + Convert" is pressed. `index` is 1-based.
function glue.compare_convert_tree(theirs, index, label, keepCopy)
	local tab = compare_tab()
	active_entry(tab)
	local name = theirs and "rightVersionSelect" or "leftVersionSelect"
	in_mode(tab, "TREE", function()
		local control = tab.controls[name]
		require_usable(control)
		local entry = control.list[index]
		if entry == nil then
			error("no such choice", 0)
		end
		if entry.label ~= label then
			error("the list changed; pick again", 0)
		end
		glue.in_popups(function()
			local depth = #pob.main.main.popups
			control:SetSel(index)
			-- the spec's own version opens nothing
			if #pob.main.main.popups ~= depth + 1 then
				error(NOT_OFFERED, 0)
			end
			local popup = pob.main.main.popups[1]
			local button = popup.controls[keepCopy and "convertCopy" or "convert"]
			button.onClick()
		end)
	end)
	sync(tab)
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

-- ---- Items, skills and config (R9) ----

-- The rows of the Items view: PoB's own slot list (`DrawItems` builds it from its base slots,
-- `ShouldShowRing3` and `AddAbyssSockets`) and its jewel slots (`GetJewelComparisonSlots`, by
-- node). Both are taken from a `DrawItems` call, which stops once it has them.
local STOP = {}

local function item_slots(tab, entry)
	local base, jewels
	local add_abyss = tab.AddAbyssSockets
	tab.AddAbyssSockets = function(self, comparison, dest, both)
		add_abyss(self, comparison, dest, both)
		base = dest
	end
	local jewel_slots = tab.GetJewelComparisonSlots
	tab.GetJewelComparisonSlots = function(self, ...)
		jewels = jewel_slots(self, ...)
		error(STOP, 0)
	end
	local width, height = GetScreenSize()
	local ok, err = pcall(tab.DrawItems, tab, { x = 0, y = 0, width = width, height = height }, entry, {})
	tab.AddAbyssSockets = nil
	tab.GetJewelComparisonSlots = nil
	if not ok and err ~= STOP then
		error(err, 0)
	end
	if not (base and jewels) then
		error("PoB's Items view lists no slots", 0)
	end
	local primary = tab.primaryBuild
	local env = primary.calcsTab and primary.calcsTab.mainEnv
	local hasRing3 = env and env.modDB:Flag(nil, "AdditionalRingSlot")
	local rows = {}
	local function add(slot, label, pItem, cItem, missing, pWarn, cWarn)
		rows[#rows + 1] = {
			slot = slot, label = label, pItem = pItem, cItem = cItem,
			-- the Equip button is "Missing slot" (TradeHelpers.lua:418)
			missing = missing and true or false,
			-- "(tree missing allocated node)" (CompareTab.lua:3935-3936)
			pWarn = pWarn and true or false, cWarn = cWarn and true or false,
		}
	end
	local pTab, cTab = primary.itemsTab, entry.itemsTab
	for _, name in ipairs(base) do
		local pSlot, cSlot = pTab.slots[name], cTab.slots[name]
		add(name, name, pSlot and pTab.items[pSlot.selItemId], cSlot and cTab.items[cSlot.selItemId],
			name == "Ring 3" and not hasRing3)
	end
	for _, jewel in ipairs(jewels) do
		add(jewel.cSlotName, jewel.label, jewel.pItem, jewel.cItem, false,
			jewel.pItem and not jewel.pNodeAllocated, jewel.cItem and not jewel.cNodeAllocated)
	end
	return rows
end

local function item_rows(tab, entry)
	local out = {}
	for i, row in ipairs(item_slots(tab, entry)) do
		out[i] = {
			slot = row.slot,
			label = row.label,
			ours = row.pItem and row.pItem.id or nil,
			theirs = row.cItem and row.cItem.name or nil,
			differ = (row.pItem and row.pItem.raw) ~= (row.cItem and row.cItem.raw),
			can_copy = row.cItem ~= nil,
			can_equip = row.cItem ~= nil and not row.missing,
			diff_label = trade_helpers.getSlotDiffLabel(row.pItem, row.cItem),
			ours_unallocated = row.pWarn,
			theirs_unallocated = row.cWarn,
			-- the names are drawn in their rarity's colour (TradeHelpers.lua:470-471)
			ours_name = row.pItem and row.pItem.name or nil,
			ours_colour = trade_helpers.getRarityColor(row.pItem),
			theirs_colour = trade_helpers.getRarityColor(row.cItem),
		}
	end
	return out
end

-- The Skills view as `DrawSkills` draws it: its group pairs (PoB matches the groups by their
-- gems), each a header per side and the gem lines under it. A header is drawn at height 18 and
-- the pair's `y`; this build's side is the left column.
local function skill_rows(tab, entry)
	local strings = {}
	local draw_string = DrawString
	DrawString = function(x, y, align, height, font, text)
		strings[#strings + 1] = { x = x, y = y, height = height, text = text }
	end
	local width, height = GetScreenSize()
	local ok, err = pcall(tab.DrawSkills, tab, { x = 0, y = 0, width = width, height = height }, entry)
	DrawString = draw_string
	if not ok then
		error(err, 0)
	end
	-- the first two are the builds' names, drawn `colWidth` apart in the tab's own coordinates
	-- (CompareTab.lua:4370-4371); the rows are drawn in the list's viewport, the comparison's
	-- column from `colWidth` on (CompareTab.lua:4446-4454)
	local right = strings[2] and (strings[2].x - strings[1].x) or 0
	local rows, row = {}, nil
	for i = 3, #strings do
		local s = strings[i]
		local side = s.x < right and "ours" or "theirs"
		if s.height == 18 then
			if not row or row.y ~= s.y then
				row = { y = s.y, ours = {}, theirs = {} }
				rows[#rows + 1] = row
			end
			row[side .. "_header"] = s.text
		else
			table.insert(row[side], s.text)
		end
	end
	local added, missing = colorCodes.POSITIVE .. "+ ", colorCodes.NEGATIVE .. "- "
	local out = {}
	for i, r in ipairs(rows) do
		local differ = not (r.ours_header and r.theirs_header)
		for _, list in ipairs({ r.ours, r.theirs }) do
			for _, line in ipairs(list) do
				if line:sub(1, #added) == added or line:sub(1, #missing) == missing then
					differ = true
				end
			end
		end
		out[i] = {
			label = r.ours_header or r.theirs_header, theirs_label = r.theirs_header,
			ours = r.ours, theirs = r.theirs, differ = differ,
		}
	end
	return out
end

-- The Config view's rows as `LayoutConfigView` lists them (CompareTab.lua:2100-2136): per
-- section, the options the builds set differently, then the ones they set alike that are
-- relevant to either build (all of them, but PoB's excluded ones, with "Show All
-- Configurations" on). Values are PoB's `FormatConfigValue` texts. The sections are the ones
-- `RebuildConfigControls` keeps, rebuilt when the Config view would rebuild them
-- (CompareTab.lua:2025-2029). PoB's search box is ours (the UI filters the labels).
local function config_rows(tab, entry)
	if tab.configCompareId ~= tab.activeCompareIndex or tab.configNeedsRebuild then
		tab:RebuildConfigControls(entry)
		tab.configCompareId = tab.activeCompareIndex
		tab.configNeedsRebuild = false
	end
	local pInput = tab.primaryBuild.configTab.input or {}
	local cInput = entry.configTab.input or {}
	local out = {}
	local function row(varData, common)
		local var = varData.var
		out[#out + 1] = {
			var = var,
			label = varData.label or var,
			ours = tab:FormatConfigValue(varData, pInput[var]),
			theirs = tab:FormatConfigValue(varData, cInput[var]),
			common = common,
		}
	end
	for _, section in ipairs(tab.configSections) do
		local commons = {}
		for _, info in ipairs(section.items) do
			local varData = info.varData
			local var = varData.var
			local pVal, cVal = tab:NormalizeConfigVals(varData, pInput[var], cInput[var])
			if tostring(pVal) ~= tostring(cVal) then
				row(varData, false)
			else
				local relevant = config_visibility.isRelevantForBuild(varData, tab.primaryBuild)
					or config_visibility.isRelevantForBuild(varData, entry)
				if relevant or (tab.configToggle and not config_visibility.isShowAllExcluded(varData)) then
					commons[#commons + 1] = varData
				end
			end
		end
		for _, varData in ipairs(commons) do
			row(varData, true)
		end
	end
	return out
end

-- The comparison's item in `slot`, as hovering it shows: PoB's `AddItemTooltip` on the
-- comparison's Items tab (CompareTab.lua:3962-3965). Nil when there is none.
function glue.compare_item_tooltip(slot)
	if not slot or not glue.build_loaded() then
		return nil
	end
	local tab = pob.build.compareTab
	local entry = sync(tab)
	if not entry then
		return nil
	end
	for _, row in ipairs(item_slots(tab, entry)) do
		if row.slot == slot and row.cItem then
			-- the tab's viewport is the screen less the sidebar (Build.lua:1268-1273)
			local width = GetScreenSize() - 312
			local tooltip = new("Tooltip")
			entry.itemsTab:AddItemTooltip(tooltip, row.cItem, nil, nil, math.min(600, math.max(260, width - 24)))
			return glue.read_tooltip(tooltip)
		end
	end
	return nil
end

-- PoB's Copy and Equip buttons on an Items row (CompareTab.lua:3936-3941)
function glue.compare_copy_item(slot, andUse)
	local tab = compare_tab()
	local entry = active_entry(tab)
	for _, row in ipairs(item_slots(tab, entry)) do
		if row.slot == slot then
			if not row.cItem or (andUse and row.missing) then
				error(NOT_OFFERED, 0)
			end
			glue.in_popups(function()
				tab:CopyCompareItemToPrimary(slot, entry, andUse)
			end)
			return
		end
	end
	error("no such slot", 0)
end

-- The comparison's item on an Items row, for its "Buy" button: PoB passes it with the row's
-- slot (CompareTab.lua:3807-3808, 3949-3951). A row without one isn't offered.
function glue.compare_buy_item(slot)
	local tab = compare_tab()
	local entry = active_entry(tab)
	for _, row in ipairs(item_slots(tab, entry)) do
		if row.slot == slot then
			if not row.cItem then
				error(NOT_OFFERED, 0)
			end
			return row.cItem
		end
	end
	error("no such slot", 0)
end

-- The Config view's "Copy Config from Compare Build", then an undo state of ours (R8)
function glue.compare_copy_config()
	local tab = compare_tab()
	active_entry(tab)
	in_mode(tab, "CONFIG", function()
		local button = tab.controls.copyConfigBtn
		require_usable(button)
		glue.in_popups(button.onClick)
	end)
	pob.build.configTab:AddUndoState()
end

-- Not side-effect free: like PoB's Draw it rebuilds a flagged comparison and refills the
-- selectors (`sync`), and it takes the one-shot `switched`.
function glue.compare_view()
	local tab = pob.build.compareTab
	if not tab or #tab.compareEntries == 0 then
		return nil
	end
	local switch = switched == tab
	switched = nil
	local entry = active_entry(tab)
	local entries = {}
	for i, compareEntry in ipairs(tab.compareEntries) do
		entries[i] = compareEntry.label or ("Build " .. i)
	end
	local controls = {}
	for _, spec in ipairs(CONTROLS) do
		local name, mode = spec[1], spec[2]
		in_mode(tab, mode, function()
			local view = glue.describe_control(name, tab.controls[name])
			if name == "configToggleBtn" then
				view.checked = tab.configToggle and true or false
			end
			controls[#controls + 1] = view
		end)
	end
	local rows, headers = summary(tab, entry)
	return {
		entries = entries,
		active = tab.activeCompareIndex - 1,
		headers = headers,
		controls = controls,
		summary = rows,
		items = item_rows(tab, entry),
		skills = skill_rows(tab, entry),
		config = config_rows(tab, entry),
		switched = switch,
	}
end
