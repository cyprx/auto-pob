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

local function rows_for(build, list, actor, output)
	local rows = {}
	for _, data in ipairs(list) do
		if data.stat and not data.childStat and data.stat ~= "SkillDPS" and not data.hideStat
			and match_flags(data.flag, data.notFlag, actor.mainSkill.skillFlags) then
			local value = output[data.stat]
			if type(value) == "number"
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

function glue.stats_view()
	local build = pob.build
	local calcs = build.calcsTab
	local env = calcs.mainEnv
	local result = {}
	local player = rows_for(build, build.displayStats, env.player, calcs.mainOutput)
	if #player > 0 then result.stats = player end
	local minionOutput = calcs.mainOutput.Minion
	if minionOutput and env.minion then
		local minion = rows_for(build, build.minionDisplayStats, env.minion, minionOutput)
		if #minion > 0 then result.minion = minion end
	end
	return result
end
