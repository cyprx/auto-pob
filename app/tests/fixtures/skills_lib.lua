-- Helpers for the Skills recipes. Runs in unmodified upstream PoB after a build is loaded.
-- Written independently of glue/skills.lua: each group goes through PoB's own
-- `SetDisplayGroup`, and the values are read from the controls it syncs. Nothing here may call
-- our glue.

local function shown(control)
	return control:IsShown() and true or false
end

local function labels(control)
	local out = {}
	for i, item in ipairs(control.list) do
		out[i] = type(item) == "table" and item.label or item
	end
	return out
end

local function dropdown(control, selected)
	return { labels = labels(control), selected = selected - 1, enabled = control:IsEnabled() and true or false }
end

local function selIndexByValue(control, key, value)
	for i, item in ipairs(control.list) do
		if item[key] == value then return i end
	end
	return 1
end

local function toggle(control)
	if not shown(control) then return nil end
	return { label = control:GetProperty("label"), on = control.state and true or false }
end

local function groupDump(tab, index, group)
	tab:SetDisplayGroup(group)
	local c = tab.controls
	local out = {
		label = c.groupLabel.buf,
		display_label = c.groupList:GetRowValue(1, index, group),
		slot = c.groupSlot:GetSelValue().slotName,
		enabled = c.groupEnabled.state and true or false,
		slot_enabled = group.slotEnabled ~= false,
		full_dps = c.includeInFullDPS.state and true or false,
		source = group.source,
		is_main = build.mainSocketGroup == index,
		can_optimise = shown(c.optimiseSockets),
		gems = {},
	}
	if shown(c.groupCount) then out.count = c.groupCount.buf end
	if group.source ~= nil then out.source_note = c.sourceNote.label() end
	if shown(c.imbuedSupportLabel) then
		out.imbued = { name = group.imbuedSupport, enabled = c.imbuedSupport:IsEnabled() and true or false }
	end
	if shown(c.socketsLabel) then out.sockets_line = c.socketsLabel.label() end
	for i, gem in ipairs(group.gemList) do
		local slot = tab.gemSlots[i]
		local effect = gem.gemData and gem.gemData.grantedEffect or gem.grantedEffect
		out.gems[i] = {
			name_spec = slot.nameSpec.buf,
			gem_id = gem.gemId,
			level = tonumber(slot.level.buf),
			quality = tonumber(slot.quality.buf),
			enabled = slot.enabled.state and true or false,
			count = shown(slot.count) and tonumber(slot.count.buf) or nil,
			colour = gem.color,
			support = effect ~= nil and effect.support and true or false,
			matches_socket = gem.matchesSocket and true or false,
			error = gem.errMsg,
			toggle1 = toggle(slot.enableGlobal1),
			toggle2 = toggle(slot.enableGlobal2),
		}
	end
	return out
end

-- The Skills tab as plain tables
function skillsDump()
	local tab = build.skillsTab
	local c = tab.controls
	local out = { sets = {}, active_set = tab.activeSkillSetId, groups = {}, slot_choices = {} }
	for i, id in ipairs(tab.skillSetOrderList) do
		out.sets[i] = { id = id, title = tab.skillSets[id].title or "Default" }
	end
	for i, choice in ipairs(c.groupSlot.list) do
		out.slot_choices[i] = { label = choice.label, slot = choice.slotName }
	end
	for i, group in ipairs(tab.socketGroupList) do
		out.groups[i] = groupDump(tab, i, group)
	end
	local help = {}
	for i, item in ipairs(c.defaultLevel.list) do help[i] = item.description end
	out.options = {
		sort_by_dps = tab.sortGemsByDPS and true or false,
		sort_field = dropdown(c.sortGemsByDPSFieldControl,
			selIndexByValue(c.sortGemsByDPSFieldControl, "type", tab.sortGemsByDPSField)),
		default_level = dropdown(c.defaultLevel, selIndexByValue(c.defaultLevel, "gemLevel", tab.defaultGemLevel)),
		default_level_help = help,
		default_quality = tostring(tab.defaultGemQuality or 0),
		support_types = dropdown(c.showSupportGemTypes,
			selIndexByValue(c.showSupportGemTypes, "show", tab.showSupportGemTypes)),
		show_legacy = tab.showLegacyGems and true or false,
	}
	return out
end
