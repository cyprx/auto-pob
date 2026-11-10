-- The passive tree for the Rust host: its geometry (rarely changes) and the build's state
-- on it (changes with every edit). Runs after bootstrap.lua, which exposes `pob.build`.

glue = glue or {}

-- Sprite sheets used by glue.tree_layout(), as paths relative to the PoB folder.
local sheets, sheetIndex

-- Looks up a sprite the way PoB's PassiveTree does, returning its sheet and pixel
-- rectangle. PoB's own UVs can't be used: headless images report a 1x1 size.
local function sprite(spriteType, name)
	local tree = pob.build.spec.tree
	local data = tree.skillSprites[spriteType]
	if not data or not name then
		return nil
	end
	if tree.imageZoomLevels then
		-- trees with several zoom levels: pick the one PoB draws with
		data = treeVersions[tree.treeVersion].num >= 3.19 and (data[0.3835] or data[1]) or data[#data]
	end
	local coords = data.coords[name]
	if not coords then
		return nil
	end
	-- some coordinates are per-zoom lists; PoB uses the last entry
	local function last(v)
		return type(v) == "table" and v[#v] or v
	end
	local file = data.filename:gsub("%?%x+$", ""):gsub(".*/", "")
	if not sheetIndex[file] then
		table.insert(sheets, "TreeData/" .. tree.treeVersion .. "/" .. file)
		sheetIndex[file] = #sheets
	end
	return { sheet = sheetIndex[file], x = last(coords.x), y = last(coords.y), w = last(coords.w), h = last(coords.h) }
end

-- Frames drawn around node icons, by node type (from PoB's PassiveTree nodeOverlay)
local frames = {
	Normal = { "PSSkillFrame", "PSSkillFrameActive" },
	Notable = { "NotableFrameUnallocated", "NotableFrameAllocated" },
	Keystone = { "KeystoneFrameUnallocated", "KeystoneFrameAllocated" },
	Socket = { "JewelFrameUnallocated", "JewelFrameAllocated" },
}

-- Icon and frame sprites for a node, as { unallocated, allocated } pairs.
local function nodeArt(node)
	if node.type == "Mastery" then
		return {
			icon = { sprite("masteryInactive", node.inactiveIcon), sprite("masteryActiveSelected", node.activeIcon) },
		}
	end
	local kind = node.type:lower()
	local frame = frames[node.type]
	return {
		icon = { sprite(kind .. "Inactive", node.icon), sprite(kind .. "Active", node.icon) },
		frame = frame and { sprite("frame", frame[1]), sprite("frame", frame[2]) },
	}
end

-- Nodes of the active spec that have a position: the main tree, ascendancies and any
-- cluster jewel subgraphs.
local function placed_nodes()
	local list = {}
	for id, node in pairs(pob.build.spec.nodes) do
		if node.x then
			list[#list + 1] = node
		end
	end
	return list
end

-- Identifies the layout: it changes with the tree version and with cluster jewel subgraphs
-- (which add nodes), and with nothing else.
local function layout_key(nodes)
	local sum = 0
	for _, node in ipairs(nodes) do
		sum = sum + node.id
	end
	return string.format("%s:%d:%d", pob.build.spec.treeVersion, #nodes, sum)
end

function glue.tree_layout_key()
	return layout_key(placed_nodes())
end

-- Geometry of the active spec's tree, for drawing it. Positions are PoB's own, in tree units.
function glue.tree_layout()
	sheets, sheetIndex = {}, {}
	local placed = placed_nodes()
	local nodes, edges = {}, {}
	for _, node in ipairs(placed) do
		local stats = {}
		for i, line in ipairs(node.sd or {}) do
			stats[i] = line
		end
		table.insert(nodes, {
			id = node.id,
			x = node.x,
			y = node.y,
			kind = node.type,
			name = node.dn or "",
			stats = stats,
			ascendancy = node.ascendancyName,
			-- nodes on the same orbit of the same group are connected by an arc
			group = node.g,
			orbit = node.o,
			group_x = node.group and node.group.x,
			group_y = node.group and node.group.y,
			art = nodeArt(node),
		})
		for _, other in ipairs(node.linked or {}) do
			-- each connection is listed on both nodes; keep one
			if node.id < other.id and other.x then
				table.insert(edges, { node.id, other.id })
			end
		end
	end
	return {
		version = pob.build.spec.treeVersion,
		key = layout_key(placed),
		nodes = nodes,
		edges = edges,
		sheets = sheets,
	}
end

-- Ids of the allocated nodes, sorted.
function glue.alloc_ids()
	local ids = {}
	for id in pairs(pob.build.spec.allocNodes) do
		ids[#ids + 1] = id
	end
	table.sort(ids)
	return ids
end

local function ids_of(nodes)
	local ids = {}
	for i, node in ipairs(nodes) do
		ids[i] = node.id
	end
	return ids
end

-- The classes and their ascendancies as the class dropdown lists them (sorted by name,
-- from Build:UpdateClassDropdowns). Choosing a class other than the current one asks first
-- when the tree has points that don't connect to the new class (Build.lua:255).
local function class_choices(spec)
	local allocated = spec:CountAllocNodes() > 0
	local classes = {}
	for _, entry in ipairs(pob.build.controls.classDrop.list) do
		local ascendancies = {}
		for i, ascendancy in ipairs(entry.ascendancies) do
			ascendancies[i] = { id = ascendancy.ascendClassId, name = ascendancy.label }
		end
		classes[#classes + 1] = {
			id = entry.classId,
			name = entry.label,
			needs_confirm = entry.classId ~= spec.curClassId and allocated and not spec:IsClassConnected(entry.classId),
			ascendancies = ascendancies,
		}
	end
	return classes
end

-- The point counts of the top bar's pointDisplay (Build.lua:912). `used` comes from the spec
-- directly, since the display is only refreshed after a calculation; the limits are read from it.
local function points(spec)
	local used, ascendancyUsed = spec:CountAllocNodes()
	local text = (pob.build.controls.pointDisplay.str or ""):gsub("%^x%x%x%x%x%x%x", ""):gsub("%^%d", "")
	local _, available, _, ascendancyAvailable = text:match("^%s*(%d+)%s*/%s*(%d+)%s+(%d+)%s*/%s*(%d+)")
	return {
		used = used,
		available = tonumber(available) or 0,
		ascendancy_used = ascendancyUsed,
		ascendancy_available = tonumber(ascendancyAvailable) or 0,
	}
end

-- Mastery nodes that are allocated or reachable, with the effects PoB offers for each
-- (TreeTab.lua:1009: an effect already chosen on another mastery isn't offered again).
local function mastery_state(spec)
	local masteries, options = {}, {}
	for id, effect in pairs(spec.masterySelections) do
		if spec.allocNodes[id] then
			masteries[#masteries + 1] = { node = id, effect = effect }
		end
	end
	table.sort(masteries, function(a, b) return a.node < b.node end)
	for id, node in pairs(spec.nodes) do
		if node.type == "Mastery" and (node.alloc or node.path) then
			local offered = {}
			for _, effect in ipairs(node.masteryEffects or {}) do
				local assigned = isValueInTable(spec.masterySelections, effect.effect)
				if not assigned or assigned == id then
					offered[#offered + 1] = { id = effect.effect, text = table.concat(effect.stats, " / ") }
				end
			end
			options[#options + 1] = { node = id, options = offered }
		end
	end
	table.sort(options, function(a, b) return a.node < b.node end)
	return masteries, options
end

local function node_paths(spec, field, filter)
	local records = {}
	for id, node in pairs(spec.nodes) do
		if node[field] and filter(node) then
			records[#records + 1] = { node = id, path = ids_of(node[field]) }
		end
	end
	table.sort(records, function(a, b) return a.node < b.node end)
	return records
end

local function spec_entries()
	local entries = {}
	for i, spec in ipairs(pob.build.treeTab.specList) do
		entries[i] = { title = spec.title or "Default", version = spec.treeVersion, points = (spec:CountAllocNodes()) }
	end
	return entries
end

-- The build's state on the active tree.
function glue.tree_state()
	local build = pob.build
	local spec = build.spec
	local masteries, masteryOptions = mastery_state(spec)

	-- the id of the newest tree version, keeping the spec's suffix (TreeTab.lua:326)
	local convertible
	if not spec.treeVersion:match("^" .. latestTreeVersion) then
		convertible = latestTreeVersion .. (spec.treeVersion:match("^" .. latestTreeVersion .. "(.*)") or "")
	end

	return {
		version = spec.treeVersion,
		version_label = treeVersions[spec.treeVersion].display,
		convertible_to = convertible,
		class_id = spec.curClassId,
		ascendancy_id = spec.curAscendClassId,
		classes = class_choices(spec),
		points = points(spec),
		allocated = glue.alloc_ids(),
		masteries = masteries,
		mastery_options = masteryOptions,
		-- reachable nodes
		paths = node_paths(spec, "path", function(node) return not node.alloc end),
		-- allocated nodes: what deallocating each one drops
		depends = node_paths(spec, "depends", function(node) return node.alloc end),
		search = "",
		search_matches = {},
		compare = {},
		jewels = {},
		specs = spec_entries(),
		active_spec = build.treeTab.activeSpec - 1,
		power_stats = {},
		can_undo = spec.undo[2] ~= nil,
		can_redo = spec.redo[1] ~= nil,
	}
end

-- ---- Editing ----
-- Each command repeats the steps of the PoB callback that a click would run, citing it.
-- Errors use level 0.

local function main_module()
	return pob.main.main
end

local function node_by_id(id)
	local node = pob.build.spec.nodes[id]
	if not node then
		error("unknown passive node " .. tostring(id), 0)
	end
	return node
end

-- The steps every edit ends with (Build.lua:260, PassiveTreeView.lua:388).
local function finish_edit(withTitle)
	local build = pob.build
	build.spec:AddUndoState()
	if withTitle then
		build.spec:SetWindowTitleWithBuildClass()
	end
	build.buildFlag = true
end

-- The ascendancy part of a left click on an unallocated node (PassiveTreeView.lua:397-495):
-- clicking a node of another ascendancy switches to it first. Bloodlines aren't supported, and
-- the cross-class confirmation is replaced by an error: the UI asks through `SelectClass`.
local function switch_to_clicked_ascendancy(node)
	local spec = pob.build.spec
	if node.isBloodline then
		error("secondary ascendancies aren't supported yet", 0)
	end
	if not (spec.curAscendClassId == 0 or node.ascendancyName ~= spec.curAscendClassBaseName) then
		return
	end
	if spec.curSecondaryAscendClass and node.ascendancyName == spec.curSecondaryAscendClass.id then
		return
	end
	-- same class: always allowed (PassiveTreeView.lua:421)
	for ascendClassId, ascendClass in pairs(spec.curClass.classes) do
		if ascendClass.id == node.ascendancyName then
			spec:SelectAscendClass(ascendClassId)
			finish_edit(true)
			return
		end
	end
	-- another class (PassiveTreeView.lua:437)
	for classId, classData in pairs(spec.tree.classes) do
		for ascendClassId, ascendClass in pairs(classData.classes) do
			if ascendClass.id == node.ascendancyName then
				if spec:CountAllocNodes() ~= 0 and not spec:IsClassConnected(classId) then
					error("that ascendancy belongs to another class: change class first", 0)
				end
				spec:SelectClass(classId)
				spec:SelectAscendClass(ascendClassId)
				local target = spec.nodes[node.id]
				if target and not target.alloc then
					spec:AllocNode(target)
				end
				finish_edit(true)
				return
			end
		end
	end
end

-- Left click on an unallocated node (PassiveTreeView.lua:391-513). Masteries with effects go
-- through `glue.set_mastery`, which is the popup's save.
function glue.alloc_node(id)
	local spec = pob.build.spec
	local node = node_by_id(id)
	if node.alloc then
		return
	end
	if node.type == "Mastery" and node.masteryEffects then
		error("pick a mastery effect", 0)
	end
	if node.ascendancyName then
		switch_to_clicked_ascendancy(node)
	end
	if node.alloc then
		return
	end
	if not node.path then
		error("that passive isn't connected to your tree", 0)
	end
	spec:AllocNode(node)
	finish_edit(false)
end

-- Left click on an allocated node (PassiveTreeView.lua:384).
function glue.dealloc_node(id)
	local spec = pob.build.spec
	local node = node_by_id(id)
	if not node.alloc then
		return
	end
	spec:DeallocNode(node)
	finish_edit(false)
end

-- Presses the popup's button with this label (or its `confirm` control).
local function press(popup, label)
	for key, control in pairs(popup.controls) do
		if control.onClick and (control.label == label or (label == nil and key == "confirm")) then
			control.onClick()
			return true
		end
	end
	return false
end

-- The class dropdown's callback (Build.lua:255). With unconnected points it opens a popup;
-- "Connect Path" or "Continue" is pressed for `connect`.
function glue.select_class(id, connect)
	local build = pob.build
	local drop = build.controls.classDrop
	local entry
	for _, candidate in ipairs(drop.list) do
		if candidate.classId == id then
			entry = candidate
		end
	end
	if not entry then
		error("unknown class " .. tostring(id), 0)
	end
	local main = main_module()
	local popups = #main.popups
	drop.selFunc(nil, entry)
	if #main.popups > popups then
		local popup = main.popups[1]
		local pressed = press(popup, connect and "Connect Path" or nil)
		if not pressed then
			main:ClosePopup()
		end
		if build.spec.curClassId ~= id then
			error("couldn't connect your tree to that class", 0)
		end
	end
end

-- The ascendancy dropdown's callback (Build.lua:279).
function glue.select_ascendancy(id)
	local build = pob.build
	local classes = build.spec.curClass.classes
	if id ~= 0 and not classes[id] then
		error("unknown ascendancy " .. tostring(id), 0)
	end
	build.controls.ascendDrop.selFunc(nil, { ascendClassId = id })
end

-- The Reset Tree button and its popup (TreeTab.lua:133-138).
function glue.reset_tree()
	local build = pob.build
	local main = main_module()
	local popups = #main.popups
	build.treeTab.controls.reset.onClick()
	if #main.popups <= popups then
		error("couldn't reset the tree", 0)
	end
	local popup = main.popups[1]
	if not (popup.controls.reset and popup.controls.reset.onClick) then
		main:ClosePopup()
		error("couldn't reset the tree", 0)
	end
	popup.controls.reset.onClick()
end

-- SaveMasteryPopup (TreeTab.lua:992): the effect must be one OpenMasteryPopup offers
-- (TreeTab.lua:1009).
function glue.set_mastery(id, effectId)
	local build = pob.build
	local spec = build.spec
	local node = node_by_id(id)
	local offered = false
	if node.type == "Mastery" then
		for _, effect in ipairs(node.masteryEffects or {}) do
			local assigned = isValueInTable(spec.masterySelections, effect.effect)
			if effect.effect == effectId and (not assigned or assigned == node.id) then
				offered = true
			end
		end
	end
	if not offered then
		error("that mastery effect isn't available here", 0)
	end
	if not node.alloc and not node.path then
		error("that passive isn't connected to your tree", 0)
	end
	-- no popup is open, so the ClosePopup at its end has nothing to close
	build.treeTab:SaveMasteryPopup(node, { selValue = { id = effectId } })
end

-- Ctrl+Z and Ctrl+Y on the tree tab (TreeTab.lua:358-366).
function glue.tree_undo()
	pob.build.spec:Undo()
	pob.build.buildFlag = true
end

function glue.tree_redo()
	pob.build.spec:Redo()
	pob.build.buildFlag = true
end
