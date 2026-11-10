-- The Skills tab's state, as the tab's controls would show it.
-- PoB syncs the group controls with the displayed group in `SetDisplayGroup`, which also runs
-- `ProcessSocketGroup` and may rewrite gem levels. A view must not change the build, so each
-- group is shown by hand: the controls' closures only read `displayGroup` and the slot
-- dropdown's selection, so those two are set, the gem slots are updated, and both are put
-- back afterwards. Whether a control shows is `IsShown`, which also follows its anchors: a
-- gem's count and Vaal toggles hang off the delete button, which an item's group hides.

glue = glue or {}

local function drop_list(control, key_of)
	local labels = {}
	for i, item in ipairs(control.list) do
		labels[i] = type(item) == "table" and item.label or item
	end
	return labels
end

-- The 0-based index of the list entry whose `key` is `value`
local function index_by(control, key, value)
	for i, item in ipairs(control.list) do
		if type(item) == "table" and item[key] == value then return i - 1 end
	end
	return 0
end

local function drop_view(control, key, value)
	return {
		labels = drop_list(control),
		selected = index_by(control, key, value),
		enabled = control:GetProperty("enabled") ~= false,
	}
end

local function toggle_view(slot_control, gem, field)
	if not slot_control:IsShown() then return nil end
	return {
		label = slot_control:GetProperty("label"),
		on = gem[field] and true or false,
	}
end

local function gem_view(tab, group, index, gem)
	local slot = tab.gemSlots[index]
	local effect = (gem.gemData and gem.gemData.grantedEffect) or gem.grantedEffect
	return {
		name_spec = gem.nameSpec or "",
		gem_id = gem.gemId,
		level = gem.level,
		quality = gem.quality,
		enabled = gem.enabled and true or false,
		count = slot.count:IsShown() and (tonumber(gem.count) or 1) or nil,
		colour = gem.color or "",
		support = effect ~= nil and effect.support and true or false,
		matches_socket = gem.matchesSocket and true or false,
		error = gem.errMsg,
		toggle1 = toggle_view(slot.enableGlobal1, gem, "enableGlobal1"),
		toggle2 = toggle_view(slot.enableGlobal2, gem, "enableGlobal2"),
	}
end

local function group_view(tab, index, group)
	tab.displayGroup = group
	tab.controls.groupSlot:SelByValue(group.slot, "slotName")
	tab:UpdateGemSlots()
	local c = tab.controls
	local view = {
		label = group.label or "",
		display_label = c.groupList:GetRowValue(1, index, group),
		slot = group.slot,
		enabled = group.enabled and true or false,
		slot_enabled = group.slotEnabled ~= false,
		full_dps = (group.includeInFullDPS and group.enabled) and true or false,
		source = group.source,
		is_main = tab.build.mainSocketGroup == index,
		can_optimise = c.optimiseSockets:IsShown() and true or false,
		gems = {},
	}
	if c.groupCount:IsShown() then
		view.count = tostring(group.groupCount or 1)
	end
	if group.source ~= nil then
		view.source_note = c.sourceNote:GetProperty("label")
	end
	if c.imbuedSupportLabel:IsShown() then
		view.imbued = {
			name = group.imbuedSupport,
			enabled = c.imbuedSupport:GetProperty("enabled") and true or false,
		}
	end
	if c.socketsLabel:IsShown() then
		view.sockets_line = c.socketsLabel:GetProperty("label")
	end
	for i, gem in ipairs(group.gemList) do
		view.gems[i] = gem_view(tab, group, i, gem)
	end
	return view
end

local function options_view(tab)
	local c = tab.controls
	return {
		sort_by_dps = tab.sortGemsByDPS and true or false,
		sort_field = drop_view(c.sortGemsByDPSFieldControl, "type", tab.sortGemsByDPSField),
		default_level = drop_view(c.defaultLevel, "gemLevel", tab.defaultGemLevel),
		default_level_help = (function()
			local help = {}
			for i, item in ipairs(c.defaultLevel.list) do help[i] = item.description or "" end
			return help
		end)(),
		default_quality = tostring(tab.defaultGemQuality or 0),
		support_types = drop_view(c.showSupportGemTypes, "show", tab.showSupportGemTypes),
		show_legacy = tab.showLegacyGems and true or false,
	}
end

function glue.skills_view()
	local tab = pob.build.skillsTab
	local c = tab.controls
	local view = {
		sets = {},
		active_set = tab.activeSkillSetId,
		groups = {},
		slot_choices = {},
		options = options_view(tab),
		can_undo = tab.undo[2] ~= nil,
		can_redo = tab.redo[1] ~= nil,
	}
	for i, id in ipairs(tab.skillSetOrderList) do
		view.sets[i] = { id = id, title = tab.skillSets[id].title or "Default" }
	end
	for i, choice in ipairs(c.groupSlot.list) do
		view.slot_choices[i] = { label = choice.label, slot = choice.slotName }
	end
	local shown, slot_index = tab.displayGroup, c.groupSlot.selIndex
	local ok, err = pcall(function()
		for i, group in ipairs(tab.socketGroupList) do
			view.groups[i] = group_view(tab, i, group)
		end
	end)
	tab.displayGroup, c.groupSlot.selIndex = shown, slot_index
	-- the gem slots follow the group on show; make them match the restored one again
	tab:UpdateGemSlots()
	if not ok then error(err, 0) end
	return view
end
