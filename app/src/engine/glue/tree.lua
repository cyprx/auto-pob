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
-- cluster jewel subgraphs. Sorted by id, so the layout is the same wherever the nodes come from.
local function placed_nodes()
	local list = {}
	for id, node in pairs(pob.build.spec.nodes) do
		if node.x then
			list[#list + 1] = node
		end
	end
	table.sort(list, function(a, b) return a.id < b.id end)
	return list
end

-- Everything the layout draws that can differ between specs on the same tree version: the
-- version, then per node its id, name, stat lines and art names. Tattoos, timeless jewels,
-- cluster jewels and mastery effects all rewrite these on the spec's own nodes. The host
-- hashes the text into the layout's key.
local function layout_digest(nodes)
	local parts = { pob.build.spec.treeVersion }
	for _, node in ipairs(nodes) do
		parts[#parts + 1] = table.concat({
			node.id,
			node.dn or "",
			table.concat(node.sd or {}, "\1"),
			node.type or "",
			node.icon or "",
			node.inactiveIcon or "",
			node.activeIcon or "",
			node.ascendancyName or "",
		}, "\2")
	end
	return table.concat(parts, "\n")
end

-- Whether PoB's right-click opens the node replacement for this node (PassiveTreeView.lua:519-526):
-- a tattoo, a small Strength, Dexterity or Intelligence, a "+30 to" attribute notable, or a
-- keystone, and never an expansion skill. Static: it doesn't depend on what is allocated.
function glue.node_replaceable(node)
	if node.expansionSkill then
		return false
	end
	if node.isTattoo or node.type == "Keystone" then
		return true
	end
	if node.type == "Normal" then
		return node.dn == "Strength" or node.dn == "Dexterity" or node.dn == "Intelligence"
	end
	if node.type == "Notable" then
		local first = node.sd and node.sd[1]
		return first ~= nil
			and (first:match("+30 to Dexterity") ~= nil or first:match("+30 to Strength") ~= nil
				or first:match("+30 to Intelligence") ~= nil)
	end
	return false
end

-- Whether the node is a mastery with effects to choose from (PassiveTreeView.lua:529-532)
function glue.node_mastery_effects(node)
	return node.type == "Mastery" and node.masteryEffects ~= nil
end

function glue.tree_layout_digest()
	return layout_digest(placed_nodes())
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
			replaceable = glue.node_replaceable(node),
			mastery_effects = glue.node_mastery_effects(node),
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
		digest = layout_digest(placed),
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
		local classData = pob.build.spec.tree.classes[entry.classId]
		for i, ascendancy in ipairs(entry.ascendancies) do
			-- `ascend_id` is the ascendancyName nodes carry (e.g. "Raider"); the label is
			-- ascendClass.name (e.g. "Warden") and can differ from it
			ascendancies[i] = {
				id = ascendancy.ascendClassId,
				name = ascendancy.label,
				ascend_id = (classData.classes[ascendancy.ascendClassId] or {}).id or "",
			}
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

local function unallocated(node) return not node.alloc end
local function allocated(node) return node.alloc end

-- Whether `node_paths(spec, field, filter)` would give `records` again: every node it lists
-- is there with the same path (`by_id`: the paths by node id), and no other. A walk over the
-- paths costs far less than building them anew, and the view resends them after every edit.
local function same_paths(spec, field, filter, records, by_id)
	local count = 0
	for id, node in pairs(spec.nodes) do
		local nodes = node[field]
		if nodes and filter(node) then
			local held = by_id[id]
			if not held then
				return false
			end
			local n = 0
			for i, other in ipairs(nodes) do
				if held[i] ~= other.id then
					return false
				end
				n = i
			end
			if n ~= #held then
				return false
			end
			count = count + 1
		end
	end
	return count == #records
end

local function by_node(records)
	local by_id = {}
	for _, record in ipairs(records) do
		by_id[record.node] = record.path
	end
	return by_id
end

-- The tree state's reachable paths and dependants as last built, and their token: a new one
-- whenever they differ from the last ones
local paths_cache = { token = 0 }

-- The build was left: the old lists go, so the next tree state builds afresh. The token
-- keeps counting, so no list the engine still holds can match a new one.
function glue.tree_paths_forget()
	paths_cache = { token = paths_cache.token }
end

local function current_paths(spec)
	local c = paths_cache
	if c.paths
		and same_paths(spec, "path", unallocated, c.paths, c.paths_by)
		and same_paths(spec, "depends", allocated, c.depends, c.depends_by) then
		return c
	end
	c.paths = node_paths(spec, "path", unallocated)
	c.depends = node_paths(spec, "depends", allocated)
	c.paths_by = by_node(c.paths)
	c.depends_by = by_node(c.depends)
	c.token = c.token + 1
	return c
end

-- The colours the tree view gives the nodes when comparing with another spec: TreeTab.lua:445
-- picks the spec, PassiveTreeView.lua:808 and :936 colour each node. A node that keeps its
-- default colour (nil) isn't listed.
local function compare_colours(build, compareSpec)
	local viewer = build.treeTab.viewer
	local colours = {}
	if compareSpec then
		for id, node in pairs(build.spec.nodes) do
			local r, g, b = viewer:GetCompareNodeColor(node, compareSpec.nodes[id], build.spec, build, nil)
			if r then
				colours[#colours + 1] = { node = id, rgb = { r, g, b } }
			end
		end
		table.sort(colours, function(a, b) return a.node < b.node end)
	end
	return colours
end

-- With the Compare tab's Tree view on and a comparison active, the overlay goes first (R7): PoB
-- draws this build's viewer with `compareSpec` set to the comparison's spec, then clears it
-- (CompareTab.lua:3383-3405). Side by side (overlay unchecked) nothing is coloured. Returns
-- nil when the Compare tab doesn't draw the tree.
local function overlay_state(build)
	local tab = build.compareTab
	local entry = tab and tab.compareViewMode == "TREE" and tab:GetActiveCompare()
	if not entry then
		return nil
	end
	if not tab.treeOverlayMode then
		return {}
	end
	local viewer = build.treeTab.viewer
	viewer.compareSpec = entry.spec
	local ok, colours = pcall(compare_colours, build, entry.spec)
	viewer.compareSpec = nil
	if not ok then
		error(colours, 0)
	end
	return colours
end

local function compare_state(build)
	local overlay = overlay_state(build)
	if overlay then
		return overlay, nil
	end
	local tab = build.treeTab
	local compareSpec = tab.isComparing and tab.specList[tab.activeCompareSpec] or nil
	tab.viewer.compareSpec = compareSpec
	return compare_colours(build, compareSpec), compareSpec and tab.activeCompareSpec - 1 or nil
end

-- The spec the drawn tree is compared with: the comparison's in the Compare tab's overlay
-- (CompareTab.lua:3392), none side by side, else the Tree tab's (TreeTab.lua:445)
local function viewed_compare_spec(build)
	local compare = build.compareTab
	local entry = compare and compare.compareViewMode == "TREE" and compare:GetActiveCompare()
	if entry then
		return compare.treeOverlayMode and entry.spec or nil
	end
	local tab = build.treeTab
	return tab.isComparing and tab.specList[tab.activeCompareSpec] or nil
end

-- A node PoB draws although this build's spec doesn't have it: allocated in the compared spec
-- only, such as a cluster jewel's subgraph (PassiveTreeView.lua:1102-1107, 308-320)
local function compare_only(build, compareSpec, id)
	local node = compareSpec and compareSpec.nodes[id]
	return node ~= nil and not build.spec.nodes[id] and node.alloc and node.x ~= nil
		and node.y ~= nil and node.type ~= "ClassStart" and node.type ~= "AscendClassStart"
end

-- Those nodes, sorted by id
local function compare_only_nodes(build)
	local compareSpec = viewed_compare_spec(build)
	local list = {}
	if compareSpec then
		for id, node in pairs(compareSpec.nodes) do
			if compare_only(build, compareSpec, id) then
				list[#list + 1] = {
					id = id, x = node.x, y = node.y, kind = node.type, name = node.dn or "",
				}
			end
		end
		table.sort(list, function(a, b) return a.id < b.id end)
	end
	return list
end

-- The words PassiveTreeView:Draw splits the search text into (bootstrap.lua)
local search_words = AutoPobSearchWords

-- Ids of the nodes the viewer would highlight for its search text: Draw caches
-- `DoesNodeMatchSearchParams` for every node when the text changes (PassiveTreeView.lua:780-802).
-- `fresh`: only tree searches ran since a frame's Draw did that for this text and spec
-- (`glue.set_tree_search`), so its results are read rather than matched again.
local function search_matches(build, fresh)
	local viewer = build.treeTab.viewer
	local results = fresh and viewer.searchStrCached == viewer.searchStr and viewer.searchStrResults
	if not results then
		viewer.searchParams = search_words(viewer.searchStr)
	end
	local matches = {}
	if #viewer.searchParams > 0 then
		for id, node in pairs(build.spec.nodes) do
			local match
			if results then
				match = results[id]
			else
				match = viewer:DoesNodeMatchSearchParams(node)
			end
			if match then
				matches[#matches + 1] = id
			end
		end
		table.sort(matches)
	end
	return matches
end

-- The sockets that hold a jewel. The radius is the table PoB highlights on hover
-- (PassiveTreeView.lua:977-995) and the ring is drawn from `data.jewelRadius`
-- (PassiveTreeView.lua:1163); a jewel without a radius has neither.
local function jewel_sockets(build)
	local spec = build.spec
	local sockets = {}
	for id, node in pairs(spec.nodes) do
		local jewel = node.type == "Socket" and spec:GetSocketedJewel(id)
		if jewel then
			local radiusNodes, ring = {}, nil
			local index = jewel.jewelRadiusIndex
			local rings = build.data.jewelRadius[index]
			if index and rings and node.nodesInRadius and node.nodesInRadius[index] then
				for radiusId in pairs(node.nodesInRadius[index]) do
					radiusNodes[#radiusNodes + 1] = radiusId
				end
				table.sort(radiusNodes)
				ring = { rings.inner, rings.outer }
			end
			sockets[#sockets + 1] = { node = id, name = jewel.title or jewel.name or "", radius_nodes = radiusNodes, ring = ring }
		end
	end
	table.sort(sockets, function(a, b) return a.node < b.node end)
	return sockets
end

local function power_stat_labels(tab)
	local labels = {}
	for i, stat in ipairs(tab.powerStatList) do
		labels[i] = stat.label
	end
	return labels
end

local function spec_entries()
	local entries = {}
	for i, spec in ipairs(pob.build.treeTab.specList) do
		entries[i] = { title = spec.title or "Default", untitled = spec.title == nil, version = spec.treeVersion, points = (spec:CountAllocNodes()) }
	end
	return entries
end

-- The Version dropdown's entries (TreeTab.lua:156-175), in PoB's order (NF-7)
local function version_choices(tab)
	local choices = {}
	for i, entry in ipairs(tab.treeVersions) do
		choices[i] = { label = entry.label, value = entry.value }
	end
	return choices
end

-- The build's state on the active tree. `fresh`: see `search_matches`.
-- `held`: the token of the paths the caller holds. When they are still current they are left
-- out; the second and third results are their token and whether they were sent.
function glue.tree_state(fresh, held)
	local build = pob.build
	local spec = build.spec
	local paths = current_paths(spec)
	local send = held ~= paths.token
	local masteries, masteryOptions = mastery_state(spec)
	local compare, compareSpec = compare_state(build)

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
		bloodline = spec.curSecondaryAscendClass and spec.curSecondaryAscendClass.id or nil,
		classes = class_choices(spec),
		points = points(spec),
		allocated = glue.alloc_ids(),
		masteries = masteries,
		mastery_options = masteryOptions,
		-- reachable nodes
		paths = send and paths.paths or nil,
		-- allocated nodes: what deallocating each one drops
		depends = send and paths.depends or nil,
		search = build.treeTab.viewer.searchStr,
		search_matches = search_matches(build, fresh),
		compare = compare,
		compare_spec = compareSpec,
		compare_only = compare_only_nodes(build),
		jewels = jewel_sockets(build),
		specs = spec_entries(),
		active_spec = build.treeTab.activeSpec - 1,
		power_stats = power_stat_labels(build.treeTab),
		heatmap = build.treeTab.viewer.showHeatMap and build.calcsTab.powerStat and build.calcsTab.powerStat.label or nil,
		max_depth = build.calcsTab.nodePowerMaxDepth,
		can_undo = spec.undo[2] ~= nil,
		can_redo = spec.redo[1] ~= nil,
		heatmap_shown = build.treeTab.viewer.showHeatMap and true or false,
		show_stat_differences = build.treeTab.viewer.showStatDifferences and true or false,
		versions = version_choices(build.treeTab),
	}, paths.token, send
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
-- clicking a node of another ascendancy or bloodline switches to it first. The cross-class
-- confirmation is replaced by an error: the UI asks through `SelectClass`.
local function switch_to_clicked_ascendancy(node)
	local spec = pob.build.spec
	-- a bloodline other than the chosen one is chosen (PassiveTreeView.lua:398-411)
	if node.isBloodline and spec.tree.alternate_ascendancies then
		if not spec.curSecondaryAscendClass or node.ascendancyName ~= spec.curSecondaryAscendClass.id then
			for bloodlineId, bloodline in pairs(spec.tree.alternate_ascendancies) do
				if bloodline.id == node.ascendancyName then
					spec:SelectSecondaryAscendClass(bloodlineId)
					finish_edit(true)
					break
				end
			end
		end
		return
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

-- Runs `fn`, then closes every popup it left open, also when it fails.
local function in_popups(fn)
	local main = main_module()
	local depth = #main.popups
	local ok, err = pcall(fn)
	while #main.popups > depth do
		main:ClosePopup()
	end
	if not ok then
		error(err, 0)
	end
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
	in_popups(function()
		local popups = #main.popups
		drop.selFunc(nil, entry)
		if #main.popups > popups then
			-- a popup that isn't pressed is closed by in_popups, as Escape would
			press(main.popups[1], connect and "Connect Path" or nil)
			if build.spec.curClassId ~= id then
				error("couldn't connect your tree to that class", 0)
			end
		end
	end)
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

-- The Reset window's "Remove All Tattoos" (TreeTab.lua:140-148): it adds no undo state.
function glue.remove_all_tattoos()
	local build = pob.build
	local main = main_module()
	in_popups(function()
		local popups = #main.popups
		build.treeTab.controls.reset.onClick()
		if #main.popups <= popups then
			error("couldn't remove the tattoos", 0)
		end
		local button = main.popups[1].controls.removeTattoo
		if not (button and button.onClick) then
			error("couldn't remove the tattoos", 0)
		end
		button.onClick()
	end)
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

-- ---- Trees (specs) ----

local function spec_at(index)
	local spec = pob.build.treeTab.specList[index]
	if not spec then
		error("unknown tree " .. tostring(index - 1), 0)
	end
	return spec
end

-- Opens the Manage Passive Trees popup (TreeTab.lua:630), whose list control holds the
-- New, Rename and Delete steps, and calls `fn(list)`.
local function with_spec_list(fn)
	in_popups(function()
		pob.build.treeTab:OpenSpecManagePopup()
		for _, control in pairs(main_module().popups[1].controls) do
			if control.controls and control.controls.new and control.OnSelDelete then
				return fn(control)
			end
		end
		error("couldn't open the tree list", 0)
	end)
end

-- Types a name into the rename popup that `RenameSpec` opened and saves it
-- (PassiveSpecListControl.lua:46-69): the box drops what its filter rejects, and Save stays
-- disabled until the name has a visible character.
local function save_name(title)
	glue.save_name("tree", title)
end

-- The New button and the rename popup's Save (PassiveSpecListControl.lua:31-58). The new tree
-- is added at the end and not selected.
function glue.new_spec(title)
	with_spec_list(function(list)
		list.controls.new.onClick()
		save_name(title)
	end)
end

function glue.rename_spec(index, title)
	local spec = spec_at(index)
	with_spec_list(function(list)
		list:RenameSpec(spec, "Rename Tree")
		save_name(title)
	end)
end

-- PassiveSpecListClass:OnSelDelete and its confirmation (PassiveSpecListControl.lua:87-101).
function glue.delete_spec(index)
	if #pob.build.treeTab.specList <= 1 then
		error("the last tree can't be deleted", 0)
	end
	local spec = spec_at(index)
	with_spec_list(function(list)
		list:OnSelDelete(index, spec)
		if not press(main_module().popups[1], nil) then
			error("couldn't delete the tree", 0)
		end
	end)
end

-- The tree dropdown's callback (TreeTab.lua:39-43).
function glue.select_spec(index)
	spec_at(index)
	pob.build.modFlag = true
	pob.build.treeTab:SetActiveSpec(index)
end

-- The Compare checkbox and dropdown (TreeTab.lua:106-122); nil switches comparing off.
function glue.set_compare_spec(index)
	local controls = pob.build.treeTab.controls
	if index == nil then
		controls.compareCheck.state = false
		controls.compareCheck.changeFunc(false)
		return
	end
	local spec = spec_at(index)
	controls.compareCheck.state = true
	controls.compareCheck.changeFunc(true)
	controls.compareSelect.selFunc(index, spec)
end

local function check_version(version)
	if not treeVersions[version] then
		error("unknown tree version " .. tostring(version), 0)
	end
end

-- The version dropdown opens the convert popup (TreeTab.lua:166); Convert replaces the tree
-- and Copy + Convert keeps the old one (TreeTab.lua:649-666).
function glue.convert_spec(version, keepCopy)
	check_version(version)
	if version == pob.build.spec.treeVersion then
		error("the tree already uses that version", 0)
	end
	in_popups(function()
		pob.build.treeTab.controls.versionSelect.selFunc(nil, { value = version })
		if not press(main_module().popups[1], keepCopy and "Copy + Convert" or "Convert") then
			error("couldn't convert the tree", 0)
		end
	end)
end

-- The "Convert all trees" button's popup (TreeTab.lua:668-681).
function glue.convert_all_specs(version)
	check_version(version)
	in_popups(function()
		pob.build.treeTab:OpenVersionConvertAllPopup(version)
		if not press(main_module().popups[1], "Convert") then
			error("couldn't convert the trees", 0)
		end
	end)
end

-- The short link a finished PoEURL shrink produced, until it is taken (R20).
local short_link

function glue.short_link_ready()
	return short_link ~= nil
end

function glue.take_short_link()
	local link = short_link
	short_link = nil
	return link
end

-- A request that presses `control` of a detached popup (R20, R11). The popup belongs to the
-- request, so PoB's callbacks still find it on `main.popups` when they run, close it
-- themselves (a tree that was added) or leave their message on it. Returns the request id.
local function press_in_popup(label, popup, control, opts)
	if not control:IsShown() or not control:IsEnabled() then
		main_module():ClosePopup()
		error("PoB doesn't offer that here", 0)
	end
	opts.popup = popup
	local id, message = glue.net_begin(label, opts, control.onClick)
	if not id then
		error(message or "couldn't start the request", 0)
	end
	return id
end

-- The Import button on a poeurl.com link resolves it in a subscript (TreeTab.lua:741-771).
local function resolve_short_link(tab, title, url)
	tab:OpenImportPopup()
	local popup = main_module().popups[1]
	local controls = popup.controls
	-- typed in, so the Import button enables itself
	glue.type_into(controls.name, title)
	glue.type_into(controls.edit, url)
	if not controls.import.enabled then
		main_module():ClosePopup()
		error("the tree needs a name and a link", 0)
	end
	return press_in_popup("Resolving PoEURL link", popup, controls.import, {})
end

-- The Export popup's "Shrink with PoEURL" button (TreeTab.lua:813-840). When the request has
-- finished, the popup's link box holds the short link.
function glue.shrink_tree_link()
	short_link = nil
	local tab = pob.build.treeTab
	tab:OpenExportPopup()
	local popup = main_module().popups[1]
	local controls = popup.controls
	return press_in_popup("Shrinking tree link", popup, controls.shrink, {
		done = function(req)
			if controls.shrink.label == "Done" and not req.error and controls.edit.buf:match("^http://poeurl%.com/") then
				short_link = controls.edit.buf
			end
		end,
	})
end

-- The Import Tree popup (TreeTab.lua:682-811) adds a tree and activates it. Its Import button
-- picks the link format; poeurl.com links are resolved over the network and return the
-- request's id, the others return nothing once the tree is added.
function glue.import_tree_link(title, url)
	if not title:match("%S") then
		error("the tree needs a name", 0)
	end
	if not url:match("%S") then
		error("paste a tree link", 0)
	end
	local tab = pob.build.treeTab
	if url:match("poeurl%.com/") then
		return resolve_short_link(tab, title, url)
	end
	local count, active = #tab.specList, tab.activeSpec
	local function plain(text)
		return (tostring(text or ""):gsub("%^x%x%x%x%x%x%x", ""):gsub("%^%d", ""))
	end
	in_popups(function()
		tab:OpenImportPopup()
		local controls = main_module().popups[1].controls
		glue.type_into(controls.name, title)
		glue.type_into(controls.edit, url)
		-- the boxes drop what their filter rejects, and Import stays off without a visible name
		if not controls.import.enabled then
			error("the tree needs a name and a link", 0)
		end
		local ok, err = pcall(controls.import.onClick)
		if ok and #tab.specList > count then
			return
		end
		-- a click that failed after adding the tree must not leave it behind
		if #tab.specList > count then
			while #tab.specList > count do
				table.remove(tab.specList)
			end
			tab:SetActiveSpec(active)
		end
		local message = plain(controls.msg.label)
		if message == "" and not ok then
			message = plain(tostring(err):match("[^\n]+"))
		end
		error(message ~= "" and message or "that isn't a valid tree link", 0)
	end)
end

-- The link OpenExportPopup shows (TreeTab.lua:813).
function glue.tree_link()
	local spec = pob.build.spec
	return spec:EncodeURL(treeVersions[spec.treeVersion].url)
end

-- ---- Search and tooltips ----

-- The treeSearch box's callback (TreeTab.lua:180); TreeTab:Draw copies the text back into the box.
-- Returns whether the tree view's search results are current for the build's spec: this
-- frame's Draw matched every node for the new text (PassiveTreeView.lua:780-802), or the text
-- is unchanged and `fresh` says they were current before (only tree searches ran since).
function glue.set_tree_search(text, fresh)
	-- a quoted word goes into a Lua pattern unescaped (PassiveTreeView.lua:789): one PoB can't
	-- read would raise in its Draw at every frame while the text stays, so it is refused, and
	-- the last search stays
	local ok, err = pcall(search_words, text)
	if not ok then
		error((tostring(err):gsub("^.-:%d+: ", "")), 0)
	end
	local viewer = pob.build.treeTab.viewer
	local before = viewer.searchStrCached
	viewer.searchStr = text
	pob.build.treeTab.searchFlag = text ~= viewer.searchStrSaved
	-- PoB works out `unsaved` in OnFrame (Build.lua:1303, which reads searchFlag). The frame
	-- doesn't set buildFlag, so nothing is recalculated and the heat map keeps running.
	glue.frame()
	return viewer.searchStrCached == text and (before ~= text or fresh == true)
end

-- What the tree draws when the mouse is over a node (PassiveTreeView.lua:1458), or nil when the
-- node is gone. A plain hover has no trace path (PassiveTreeView.lua:286-289), which is what
-- the tooltip reads besides the node; it doesn't read `hoverNode`.
--
-- `trace` is the path traced with Shift held (ids from the tree to the end), or empty: PoB's
-- trace mode then shows it (PassiveTreeView.lua:283-290, 1662, 1715-1716).
function glue.node_tooltip(id, trace)
	local build = pob.build
	local node = build.spec.nodes[id]
	local viewed = viewed_compare_spec(build)
	if not node then
		-- a node of the compared spec only has PoB's own tooltip for it (PassiveTreeView.lua:
		-- 1143-1147), except a mastery without effects
		local other = compare_only(build, viewed, id) and viewed.nodes[id]
		if not other or (other.type == "Mastery" and not other.masteryEffects) then
			return nil
		end
		local viewer = build.treeTab.viewer
		local tooltip = glue.tooltip()
		local compareSpec = viewer.compareSpec
		viewer.compareSpec = viewed
		local ok, err = pcall(viewer.AddCompareNodeTooltip, viewer, tooltip, other, build)
		viewer.compareSpec = compareSpec
		if not ok then
			error(tostring(err):match("[^\n]+") or "couldn't build the tooltip", 0)
		end
		return { id = id, lines = tooltip:lines(), trace = {} }
	end
	local tab = build.treeTab
	local viewer = tab.viewer
	local tooltip = glue.tooltip()
	local tracePath, compareSpec, traceMode = viewer.tracePath, viewer.compareSpec, viewer.traceMode
	viewer.tracePath = nil
	viewer.traceMode = false
	if trace and #trace > 0 then
		local nodes = {}
		for i, nodeId in ipairs(trace) do
			nodes[i] = build.spec.nodes[nodeId]
			if not nodes[i] then
				return nil
			end
		end
		viewer.tracePath = nodes
		viewer.traceMode = true
	end
	-- as TreeTab:Draw sets it (TreeTab.lua:445), or the Compare tab's overlay
	-- (CompareTab.lua:3392)
	viewer.compareSpec = viewed
	local ok, err = pcall(viewer.AddNodeTooltip, viewer, tooltip, node, build)
	viewer.tracePath, viewer.compareSpec, viewer.traceMode = tracePath, compareSpec, traceMode
	if not ok then
		error(tostring(err):match("[^\n]+") or "couldn't build the tooltip", 0)
	end
	return { id = id, lines = tooltip:lines(), trace = trace or {} }
end

-- ---- Keys ----

-- P on the tree (PassiveTreeView.lua:204-205); TreeTab:Draw then syncs the checkbox and the
-- stat dropdown to it (TreeTab.lua:459-460). Returns whether the heat map shows.
function glue.toggle_heatmap()
	local tab = pob.build.treeTab
	tab.viewer.showHeatMap = not tab.viewer.showHeatMap
	tab.controls.treeHeatMap.state = tab.viewer.showHeatMap
	tab.controls.treeHeatMapStatSelect.shown = tab.viewer.showHeatMap
	return tab.viewer.showHeatMap
end

-- Ctrl+D on the tree (PassiveTreeView.lua:206-207)
function glue.set_tree_stat_differences(on)
	pob.build.treeTab.viewer.showStatDifferences = on and true or false
end

-- A click with Shift held on the end of the traced path (PassiveTreeView.lua:500-506):
-- spec:AllocNode with the traced path instead of the node's own. `trace` runs from the tree
-- to `id`.
function glue.alloc_trace(id, trace)
	local spec = pob.build.spec
	local node = node_by_id(id)
	if node.alloc or not node.path then
		error("that passive isn't connected to your tree", 0)
	end
	if node.type == "Mastery" and node.masteryEffects then
		error("pick a mastery effect", 0)
	end
	local path = {}
	for i, nodeId in ipairs(trace) do
		path[i] = spec.nodes[nodeId]
		if not path[i] then
			error("that path isn't on your tree", 0)
		end
	end
	if path[#path] ~= node then
		error("that path doesn't end at the node", 0)
	end
	spec:AllocNode(node, path)
	finish_edit(false)
end

-- ---- Tree copy and reorder (NF-3) ----

-- The tree manager's Copy button and its "Copy Tree" name popup (PassiveSpecListControl.lua:
-- 13-21, 37-58): the copy is added at the end, not selected
function glue.copy_spec(index, title)
	local spec = spec_at(index)
	with_spec_list(function(list)
		list.selIndex, list.selValue = index, spec
		list.controls.copy.onClick()
		save_name(title)
	end)
	-- Save sets only the tab's modFlag; a frame refreshes `unsaved` (Build.lua:1303)
	glue.frame()
end

-- Dragging a row of the tree manager's list; its OnOrderChange (PassiveSpecListControl.lua:
-- 75-80) finds the active tree again, refreshes the Items tab's tree list and syncs loadouts
function glue.move_spec(from, to)
	spec_at(from)
	spec_at(to)
	if from == to then
		return
	end
	with_spec_list(function(list)
		glue.drop_list_row(list, from, to)
	end)
	glue.frame()
end

-- ---- PoB's "Convert to <latest>" button (TreeTab:610) ----

-- The button under an older tree (TreeTab.lua:330-337): the tree is copied into the latest
-- version next to it (ConvertToVersion with `success`, :588-612), and PoB says so in its
-- "Tree Converted" message. The message's title and lines are returned for the UI to show.
function glue.convert_spec_latest()
	local button = pob.build.treeTab.controls.specConvert
	if not (button:IsShown() and button:IsEnabled()) then
		error("PoB doesn't offer that here", 0)
	end
	local message
	in_popups(function()
		local depth = #main_module().popups
		button.onClick()
		local popups = main_module().popups
		if #popups > depth then
			local popup = popups[1]
			local lines = {}
			for _, control in ipairs(popup.controls) do
				lines[#lines + 1] = control:GetProperty("label")
			end
			message = { title = popup:GetProperty("title"), text = table.concat(lines, "\n") }
		end
	end)
	return message
end

-- ---- The canvas's Class Change question (PassiveTreeView.lua:474) ----

-- A left click on an unallocated node of another class's ascendancy, with points that don't
-- connect to that class: PoB asks "Class Change" and runs its Continue or Connect Path
-- callback (PassiveTreeView.lua:464-492), which these steps are.
function glue.alloc_changing_class(id, connect)
	local spec = pob.build.spec
	local node = node_by_id(id)
	if node.alloc or not node.ascendancyName or node.isBloodline then
		error("PoB doesn't offer that here", 0)
	end
	local targetClassId, targetAscendClassId
	for classId, classData in pairs(spec.tree.classes) do
		for ascendClassId, ascendClass in pairs(classData.classes) do
			if ascendClass.id == node.ascendancyName then
				targetClassId, targetAscendClassId = classId, ascendClassId
			end
		end
	end
	if not targetClassId or targetClassId == spec.curClassId then
		error("PoB doesn't offer that here", 0)
	end
	if spec:CountAllocNodes() == 0 or spec:IsClassConnected(targetClassId) then
		error("PoB doesn't ask here: the class changes with the click", 0)
	end
	if connect and not spec:ConnectToClass(targetClassId) then
		error("couldn't connect your tree to that class", 0)
	end
	spec:SelectClass(targetClassId)
	spec:SelectAscendClass(targetAscendClassId)
	local target = spec.nodes[id]
	if target and not target.alloc then
		spec:AllocNode(target)
	end
	finish_edit(true)
end
