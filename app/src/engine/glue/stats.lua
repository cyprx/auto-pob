-- The sidebar stats, as PoB's `buildMode:AddDisplayStatList` would list them.

glue = glue or {}

-- Mirrors the local `matchFlags` in Modules/Build.lua, which isn't reachable from here.
local function match_flags(required, forbidden, flags)
	if type(required) == "string" then required = { required } end
	if required then
		for _, flag in ipairs(required) do
			if not flags[flag] then return false end
		end
	end
	if type(forbidden) == "string" then forbidden = { forbidden } end
	if forbidden then
		for _, flag in ipairs(forbidden) do
			if flags[flag] then return false end
		end
	end
	return true
end

-- A dropdown as the views show it: its labels (colour codes kept), the 0-based selection and
-- whether it can be changed.
function glue.drop_list(control)
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

-- The "Skill DPS" stat: one line per Full DPS skill (Build.lua:1708-1740), sorted by total DPS
-- as PoB sorts the output list in place.
local function skill_dps_rows(build, data, output, rows)
	table.sort(output.SkillDPS, function(a, b) return (a.dps * a.count) > (b.dps * b.count) end)
	local overCap = output[data.overCapStat]
	if overCap and data.overCapStatCondFunc and not data.overCapStatCondFunc(output.SkillDPS, output) then
		overCap = nil
	end
	for _, skillData in ipairs(output.SkillDPS) do
		local trigger = ""
		if skillData.trigger and skillData.trigger ~= "" then
			trigger = colorCodes.WARNING .. " (" .. skillData.trigger .. ")" .. colorCodes.CUSTOM
		end
		local name = skillData.name .. trigger
		if skillData.count >= 2 then name = tostring(skillData.count) .. "x " .. name end
		local total = skillData.dps * skillData.count
		local notes = {}
		if skillData.skillPart then notes[#notes + 1] = "^8" .. skillData.skillPart end
		if skillData.source then notes[#notes + 1] = colorCodes.WARNING .. "from " .. skillData.source end
		rows[#rows + 1] = {
			key = data.stat,
			label = (StripEscapes(name)),
			value = total,
			text = StripEscapes(build:FormatStat({ fmt = ".1f", compactValue = data.compactValue }, total, overCap)),
			color = colorCodes.CUSTOM,
			notes = #notes > 0 and notes or nil,
		}
	end
end

local function rows_for(build, list, actor, output)
	local rows = {}
	for _, data in ipairs(list) do
		if data.stat and not data.childStat and not data.hideStat
			and match_flags(data.flag, data.notFlag, actor.mainSkill.skillFlags) then
			local value = output[data.stat]
			if data.stat == "SkillDPS" then
				if type(value) == "table" and (not data.condFunc or data.condFunc(value, output)) then
					skill_dps_rows(build, data, output, rows)
				end
			elseif type(value) == "number"
				and ((data.condFunc and data.condFunc(value, output)) or (not data.condFunc and value ~= 0)) then
				local overCap = output[data.overCapStat]
				if overCap and data.overCapStatCondFunc and not data.overCapStatCondFunc(value, output) then
					overCap = nil
				end
				local text = StripEscapes(build:FormatStat(data, value, overCap))
				if data.suffix and (not data.suffixCondFunc or data.suffixCondFunc(value, output)) then
					local suffix = data.suffix
					if type(suffix) == "function" then suffix = suffix(value, output) end
					if suffix then text = text .. " (" .. StripEscapes(suffix) .. ")" end
				end
				rows[#rows + 1] = {
					key = data.stat,
					label = data.label or data.stat,
					value = value,
					text = text,
					color = data.color,
				}
			end
		end
	end
	return rows
end

-- The sidebar's main-skill dropdowns. PoB syncs them with the build while the build draws, which
-- never runs here, so refresh them first (Build.lua:1267). A control shows when PoB shows it.
local function refreshed_controls()
	local build = pob.build
	build:RefreshSkillSelectControls(build.controls, build.mainSocketGroup, "")
	return build.controls
end

function glue.main_skill_view()
	local controls = refreshed_controls()
	local function drop(name)
		return controls[name].shown and glue.drop_list(controls[name]) or nil
	end
	local function text(name)
		return controls[name].shown and (controls[name].buf or "") or nil
	end
	return {
		group = glue.drop_list(controls.mainSocketGroup),
		skill = drop("mainSkill"),
		part = drop("mainSkillPart"),
		stages = text("mainSkillStageCount"),
		mines = text("mainSkillMineCount"),
		minion = drop("mainSkillMinion"),
		minion_skill = drop("mainSkillMinionSkill"),
		-- `IsShown`, not `shown`: a main skill with no groups leaves the flag stale, and the
		-- button hangs off the minion dropdown, which is hidden then (Build.lua:552, 1557-1583)
		spectre_library = controls.mainSkillMinionLibrary:IsShown() and true or false,
	}
end

-- The sidebar's "Manage Spectres..." button when PoB shows it (R18), after the refresh
function glue.sidebar_spectre_library()
	local control = refreshed_controls().mainSkillMinionLibrary
	return control:IsShown() and control or nil
end

local SIDEBAR_NAMES = {
	mainSocketGroup = "Group", mainSkill = "Skill", mainSkillPart = "Part",
	mainSkillMinion = "Minion", mainSkillMinionSkill = "Minion skill",
}

-- A sidebar dropdown; `index` is 1-based and PoB's own `selFunc` runs. A hidden list can be
-- stale and its callback assumes the main skill has it, so only shown ones can be pressed.
function glue.main_skill_select(name, index)
	if name == "mode" then error("Mode isn't available in the sidebar", 0) end
	local label = SIDEBAR_NAMES[name]
	if not label then error("the sidebar has no " .. name .. " selector", 0) end
	local controls = refreshed_controls()
	local control = controls[name]
	if not control.shown or #pob.build.skillsTab.socketGroupList == 0 then
		error(label .. " isn't available for this skill", 0)
	end
	if index < 1 or index > #control.list then error("no such choice", 0) end
	control:SetSel(index)
end

-- The stage and mine boxes filter their text with "%D" (digits only)
function glue.main_skill_set_count(name, text)
	if name ~= "mainSkillStageCount" and name ~= "mainSkillMineCount" then
		error("the sidebar has no " .. name .. " box", 0)
	end
	local controls = refreshed_controls()
	local control = controls[name]
	-- as for the dropdowns: PoB's callback assumes the main skill has the box
	if not control.shown or #pob.build.skillsTab.socketGroupList == 0 then
		error((name == "mainSkillStageCount" and "Stages" or "Mines") .. " isn't available for this skill", 0)
	end
	control:SetText((text:gsub("%D", "")), true)
end

function glue.stats_view()
	local build = pob.build
	local calcs = build.calcsTab
	local env = calcs.mainEnv
	local result = { main_skill = glue.main_skill_view() }
	local player = rows_for(build, build.displayStats, env.player, calcs.mainOutput)
	if #player > 0 then result.stats = player end
	local minionOutput = calcs.mainOutput.Minion
	if minionOutput and env.minion then
		local minion = rows_for(build, build.minionDisplayStats, env.minion, minionOutput)
		if #minion > 0 then result.minion = minion end
	end
	-- the "N Warnings" line under the stats, which PoB draws only with Options' "Show
	-- warnings" on (Build.lua:566-590); the lines are refilled in every `RefreshStatList`
	if pob.main.main.showWarnings then
		local warnings = {}
		for _, line in ipairs(build.controls.warnings.lines) do
			warnings[#warnings + 1] = tostring(line)
		end
		if #warnings > 0 then result.warnings = warnings end
	end
	return result
end
