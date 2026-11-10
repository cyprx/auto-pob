-- The gem picker: PoB's gem search (GemSelectControl:BuildList with the S and A filters), its
-- DPS sort (DPSBuilder, one coroutine resume per step) and gem tooltips. Runs after skills.lua.
--
-- The search keeps the group's table. Every step and view shows that group again
-- (`displayGroup`, which CalcOutputWithThisGem reads) and puts back what was shown, so a command
-- between two slices can't move the calculation to another group (R13). The box's
-- `sortGemsBy` holds the search's filter only during a call: left set, it would hide gems
-- from a later pick in the same box.

glue = glue or {}

-- { index, group, control, text, filter }, while a search is open
local search
local with_search
-- the progress the DPS sort last reported, for the view
local percent

local function skills_tab()
	return pob.build.skillsTab
end

-- The gem box of `target` in the shown group: a 1-based gem slot or "imbued". Refused where
-- PoB's box is hidden or disabled.
local function gem_box(group, c, target)
	if target == "imbued" then
		if not c.imbuedSupportLabel:IsShown() then
			error("an item's socket group has no imbued support", 0)
		end
		if not c.imbuedSupport:GetProperty("enabled") then
			error("the imbued support of this socket group can't be changed", 0)
		end
		return c.imbuedSupport
	end
	if group.source ~= nil then
		error("an item's socket group can't be edited", 0)
	end
	if target < 1 or target > #group.gemList + 1 then
		error("no such gem", 0)
	end
	return skills_tab().gemSlots[target].nameSpec
end

-- Opens a search, or replaces the open one (R9). Clicks the group's row, as the user does
-- before reaching its gem boxes. `filter` is nil, "support" or "grants_active_skill".
function glue.gem_search_open(index, target, text, filter)
	local group, c = glue.skills_show_group(index)
	local control = gem_box(group, c, target)
	-- the Imbued Support box draws no S or A button (GemSelectControl.lua:651)
	if control.imbuedSelect and filter ~= nil then
		error("the imbued support box has no support or active filter", 0)
	end
	-- what the box's filter (`^ %a':-`) refuses can't be typed
	text = text:gsub("\r", ""):gsub(control.filterPattern, "")
	search = { index = index, group = group, control = control, text = text, filter = filter }
	percent = nil
	-- R9: the filter, PoB's cache check, then the list for the text, as focusing and typing
	-- do (the DPS sort's own re-sort reads the text BuildList keeps)
	with_search(function()
		control:UpdateSortCache()
		control:BuildList(text)
	end)
end

function glue.gem_search_close()
	search = nil
	percent = nil
end

-- Runs `fn(control)` with the search's group shown and its filter set, and puts both back.
-- Returns nil and true when the group's table is no longer at its index (the search ended).
with_search = function(fn)
	local tab = skills_tab()
	if tab.socketGroupList[search.index] ~= search.group then
		return nil, true
	end
	local shown = tab.displayGroup
	local control = search.control
	tab.displayGroup = search.group
	control.sortGemsBy = search.filter
	local ok, result = pcall(fn, control)
	control.sortGemsBy = nil
	tab.displayGroup = shown
	if not ok then
		error(result, 0)
	end
	return result, false
end

-- Starts the DPS sort again after it failed: PoB's cache check is satisfied, so it wouldn't.
function glue.gem_dps_restart()
	if search then
		search.control.dpsBuilder = nil
		search.control.dpsBuildFlag = true
	end
end

-- Whether the search's DPS sort has work: PoB's own cache check (UpdateSortCache, which
-- notices a new output revision, changed options, group or gem) starts it again when needed,
-- then a pending start or a live coroutine is work (GemSelectControl.lua:492-509). A search
-- whose group is gone has one step left, which reports the end.
function glue.gem_dps_pending()
	if not search or search.ended then
		return false
	end
	local pending, ended = with_search(function(control)
		control:UpdateSortCache()
		return control.dpsBuildFlag or control.dpsBuilder ~= nil
	end)
	return (ended or pending) and true or false
end

-- One slice of the DPS sort: GemSelectControl:Draw's coroutine block (:492-509), after PoB's
-- own cache check (UpdateSortCache) restarts the sort when the build, the options, the group
-- or the gem changed. A failed resume is an error, where PoB swallows it outside dev mode.
function glue.gem_dps_step()
	if not search then
		return { finished = true, ended = true }
	end
	local reported
	local result, ended = with_search(function(control)
		control:UpdateSortCache()
		if control.dpsBuildFlag then
			control.dpsBuildFlag = false
			control.dpsBuilder = coroutine.create(control.DPSBuilder)
			control.dpsBuilderCallback = function(value)
				control.sortPercentage = ("%d%%"):format(value)
				reported = value
			end
			percent = nil
		end
		local co = control.dpsBuilder
		if not co then
			return { finished = true }
		end
		local ok, err = coroutine.resume(co, control)
		if not ok then
			control.dpsBuilder = nil
			error(tostring(err), 0)
		end
		if coroutine.status(co) == "dead" then
			control.dpsBuilder = nil
			return { finished = true }
		end
		return { finished = false }
	end)
	if ended then
		search.ended = true
		return { finished = true, ended = true }
	end
	if reported then
		percent = reported
	end
	result.percent = reported
	return result
end

-- The colour Draw gives a gem's name (GemSelectControl.lua:551-557)
local function name_colour(gemData)
	local color = gemData.grantedEffect.color
	if color == 1 then
		return colorCodes.STRENGTH
	elseif color == 2 then
		return colorCodes.DEXTERITY
	elseif color == 3 then
		return colorCodes.INTELLIGENCE
	end
	return "^7"
end

local function gem_row(control, key)
	local gemData = control.gems[key]
	local cache = control.sortCache
	local effect = gemData.grantedEffect
	local colour = cache.dpsColor[key]
	local row = {
		gem_id = (key:gsub("^Default:", "")),
		name = gemData.name,
		colour = name_colour(gemData),
		tags = gemData.tagString or "",
		support = effect.support and true or false,
		can_support = cache.canSupport[key] and true or false,
	}
	if colour and colour ~= "" then
		row.dps = cache.dps[key]
		row.dps_colour = colour
		-- Draw's two marks (:561-568)
		if effect.support and cache.canSupport[key] then
			row.mark = "Check"
		elseif effect.hasGlobalEffect then
			row.mark = "Plus"
		end
	end
	return row
end

-- The search's list now (R9, R11): the cache check, then BuildList for the text, so a typed
-- search is re-sorted as the DPS values come in. The "" no-match entry is left out.
function glue.gem_search_view()
	if not search then
		return { rows = {}, base_dps = 0, sorting = false, ended = true }
	end
	local view, ended = with_search(function(control)
		control:UpdateSortCache()
		control:BuildList(search.text)
		local rows = {}
		for _, key in ipairs(control.list) do
			if control.gems[key] then
				rows[#rows + 1] = gem_row(control, key)
			end
		end
		return {
			rows = rows,
			base_dps = control.sortCache.baseDPS or 0,
			sorting = (control.dpsBuilder ~= nil or control.dpsBuildFlag) and true or false,
			percent = percent,
		}
	end)
	if ended then
		return { rows = {}, base_dps = 0, sorting = false, ended = true }
	end
	return view
end

-- A gem tooltip (R15). `gem_id` set is a picker row: Draw's hover sequence (:582-604), what
-- selecting the gem would give. `gem_id` nil is the gem already in the box. The group is shown
-- as skills_view shows one (no SetDisplayGroup) and put back afterwards. The tooltip module is
-- local to GemSelectControl, so the box's own tooltip is swapped for the recorder.
function glue.gem_tooltip(index, target, gem_id)
	local tab = skills_tab()
	local group = tab.socketGroupList[index]
	if not group then
		error("no such socket group", 0)
	end
	local c = tab.controls
	local recorder = glue.tooltip()
	local shown, slot_index = tab.displayGroup, c.groupSlot.selIndex
	local ok, err = pcall(function()
		tab.displayGroup = group
		c.groupSlot:SelByValue(group.slot, "slotName")
		tab:UpdateGemSlots()
		local control
		if target == "imbued" then
			control = c.imbuedSupport
		else
			local slot = tab.gemSlots[target]
			if not slot or target > #group.gemList + 1 then
				error("no such gem", 0)
			end
			control = slot.nameSpec
		end
		local saved = control.tooltip
		control.tooltip = recorder
		local done, failure = pcall(function()
			if gem_id then
				local gemData = tab.build.data.gems[gem_id]
				if not gemData then
					error("unknown gem " .. tostring(gem_id), 0)
				end
				local calcFunc, calcBase = tab.build.calcsTab:GetMiscCalculator(control.build)
				if calcFunc then
					local output = control:CalcOutputWithThisGem(calcFunc, gemData, tab.sortGemsByDPSField == "FullDPS")
					local gemInstance = {
						level = tab:ProcessGemLevel(gemData, control.imbuedSelect),
						quality = tab.defaultGemQuality or 0,
						qualityId = "Default",
						count = 1,
						enabled = true,
						enableGlobal1 = true,
						enableGlobal2 = true,
						gemId = gemData.id,
						nameSpec = gemData.name,
						skillId = gemData.grantedEffectId,
						displayEffect = nil,
						gemData = gemData,
					}
					control:AddGemTooltip(gemInstance)
					control.tooltip:AddSeparator(10)
					tab.build:AddStatComparesToTooltip(control.tooltip, calcBase, output, "^7Selecting this gem will give you:")
				end
			else
				local gemInstance
				if target == "imbued" then
					-- Draw's imbued branch (:634-643), with the gem SetDisplayGroup would find
					if group.imbuedSupport then
						local gemData = data.gems[data.gemForBaseName[group.imbuedSupport:lower() .. " support"]]
						gemInstance = { gemData = gemData, level = 1, quality = 0 }
					end
				else
					gemInstance = group.gemList[target]
				end
				if gemInstance and gemInstance.gemData then
					control:AddGemTooltip(gemInstance)
				end
			end
		end)
		control.tooltip = saved
		if not done then
			error(failure, 0)
		end
	end)
	tab.displayGroup, c.groupSlot.selIndex = shown, slot_index
	tab:UpdateGemSlots()
	if not ok then
		error(err, 0)
	end
	return { lines = recorder:lines() }
end
