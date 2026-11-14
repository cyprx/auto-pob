-- The passive tree for the Rust host: its geometry (rarely changes) and the build's state
-- on it (changes with every edit). Runs after bootstrap.lua, which exposes `pob.build`.

glue = glue or {}

-- Image files used by glue.tree_layout(), as paths relative to the PoB folder: sprite sheets
-- and the whole files PoB loads as tree assets.
local sheets, sheetIndex

-- While set, art on a file the last layout didn't send is left out
local sheetsFrozen = false

-- ---- The tree view's art ----
-- Every image PassiveTreeView:Draw draws comes from one of PoB's tables: a sprite of a sheet
-- (a `spriteMap` entry, and the assets PoB takes from those), a sprite of a legion sheet, or a
-- whole file (the rest of `tree.assets`, loaded by PassiveTree:LoadImage). Headless images are
-- 1x1, so PoB's own sizes and texture coordinates can't be used: each table is looked up here
-- for its file and pixel rectangle. A whole file has no rectangle; the host reads its size.

-- tree -> { [PoB's table] = { path, x, y, w, h } }
local artIndexes = setmetatable({}, { __mode = "k" })

local function last(v)
	return type(v) == "table" and v[#v] or v
end

-- The file PassiveTree:LoadImage loads for a tree asset (PassiveTree.lua:883-904)
local function asset_file(tree, name)
	local path = "TreeData/" .. name .. ".png"
	local file = io.open(path, "r")
	if file then
		file:close()
		return path
	end
	return "TreeData/" .. tree.treeVersion .. "/" .. name .. ".png"
end

local function art_index(tree)
	local index = artIndexes[tree]
	if index then
		return index
	end
	index = {}
	-- the sprite sheets, at the zoom PoB loads (PassiveTree.lua:273-305)
	local versionNum = treeVersions[tree.treeVersion].num
	for spriteType, data in pairs(tree.skillSprites) do
		local maxZoom = data
		if tree.imageZoomLevels then
			maxZoom = versionNum >= 3.19 and (data[0.3835] or data[1]) or data[#data]
		end
		local path = "TreeData/" .. tree.treeVersion .. "/" .. maxZoom.filename:gsub("%?%x+$", ""):gsub(".*/", "")
		for name, coords in pairs(maxZoom.coords) do
			local sprite = tree.spriteMap[name] and tree.spriteMap[name][spriteType]
			if sprite then
				index[sprite] = { path = path, x = last(coords.x), y = last(coords.y), w = last(coords.w), h = last(coords.h) }
			end
		end
	end
	-- the legion sheets (PassiveTree.lua:365-412)
	local legion = LoadModule("TreeData/legion/tree-legion.lua")
	for _, data in ipairs(legion.treeAssets or {}) do
		for name, coords in pairs(data.coords) do
			local asset = tree.assets[name]
			if asset then
				index[asset] = { path = "TreeData/legion/" .. data.filename, x = coords.x, y = coords.y, w = coords.w, h = coords.h }
			end
		end
	end
	for spriteType, data in pairs(legion) do
		if spriteType ~= "treeAssets" then
			local maxZoom = data[#data]
			for name, coords in pairs(maxZoom.coords) do
				local sprite = tree.spriteMap[name] and tree.spriteMap[name][spriteType]
				if sprite then
					index[sprite] = { path = "TreeData/legion/" .. maxZoom.filename, x = coords.x, y = coords.y, w = coords.w, h = coords.h }
				end
			end
		end
	end
	-- whole files
	for name, data in pairs(tree.assets) do
		if not index[data] then
			index[data] = { path = asset_file(tree, name) }
		end
	end
	artIndexes[tree] = index
	return index
end

-- The file and rectangle of one of PoB's art tables, its file listed in `sheets`; nil for none,
-- or for a file the layout didn't send while the sheets are frozen
local function art(tree, data)
	if not data then
		return nil
	end
	local found = art_index(tree)[data]
	if not found then
		return nil
	end
	if not sheetIndex[found.path] then
		if sheetsFrozen then
			return nil
		end
		table.insert(sheets, found.path)
		sheetIndex[found.path] = #sheets
	end
	return { sheet = sheetIndex[found.path], x = found.x, y = found.y, w = found.w, h = found.h }
end

local function isAbyssConquered(node)
	local conqueror = node and node.conqueredBy and node.conqueredBy.conqueror
	return conqueror and conqueror.type and conqueror.type:match("^abyss_") and true or false
end

local drawStates = { "unalloc", "path", "alloc" }

-- What PassiveTreeView:Draw draws for a node in each of its states (PassiveTreeView.lua:805-904):
-- `base` and `overlay` as { unalloc, path, alloc }, `effect` as { unallocated, allocated }, and
-- `hover` the base of a hovered mastery. Which entry is drawn is the host's choice, as Draw's.
local function nodeArt(node, spec)
	local tree = spec.tree
	local assets = tree.assets
	local function a(data)
		return art(tree, data)
	end
	if node.type == "ClassStart" then
		return { overlay = { [1] = a(assets.PSStartNodeBackgroundInactive), [2] = a(assets.PSStartNodeBackgroundInactive), [3] = a(assets[node.startArt]) } }
	elseif node.type == "AscendClassStart" then
		local overlay = treeVersions[tree.treeVersion].num >= 3.10 and "AscendancyMiddle" or "PassiveSkillScreenAscendancyMiddle"
		local prefix = node.ascendancyName and tree.bloodlineSpritePrefixes and tree.bloodlineSpritePrefixes[node.ascendancyName]
		if prefix then
			overlay = prefix .. overlay
		end
		local middle = a(assets[overlay])
		return { overlay = { [1] = middle, [2] = middle, [3] = middle } }
	end
	local result = { base = {}, overlay = {}, effect = {} }
	if node.type == "Socket" then
		for i, state in ipairs(drawStates) do
			result.base[i] = a(assets[(node.name == "Charm Socket" and "Azmeri" or "") .. node.overlay[state .. (node.expansionJewel and "Alt" or "")]])
		end
	elseif node.type == "Mastery" then
		local override = spec.hashOverrides and spec.hashOverrides[node.id]
		local sprites = override and tree.spriteMap[override.icon]
		local effectSprites = override and tree.spriteMap[override.activeEffectImage]
		if node.masteryEffects then
			local ms = node.masterySprites or {}
			local inactive = (sprites and sprites.masteryInactive) or (ms.inactiveIcon and ms.inactiveIcon.masteryInactive)
			result.base[1] = a(inactive)
			result.base[2] = result.base[1]
			result.base[3] = a((sprites and sprites.masteryActiveSelected) or (ms.activeIcon and ms.activeIcon.masteryActiveSelected))
			result.hover = a((sprites and sprites.masteryConnected) or (ms.inactiveIcon and ms.inactiveIcon.masteryConnected))
			result.effect[2] = a((effectSprites and effectSprites.masteryActiveEffect) or (ms.activeEffectImage and ms.activeEffectImage.masteryActiveEffect))
		else
			local base = a((sprites and sprites.mastery) or (node.sprites and node.sprites.mastery))
			result.base = { [1] = base, [2] = base, [3] = base }
		end
	else
		if node.isTattoo and node.effectSprites then
			local effect = a(node.effectSprites.tattooActiveEffect)
			result.effect = { [1] = effect, [2] = effect }
		end
		local sprites = node.sprites or {}
		local kind = node.type:lower()
		result.base[1] = a(sprites[kind .. "Inactive"])
		result.base[2] = result.base[1]
		result.base[3] = a(sprites[kind .. "Active"])
		for i, state in ipairs(drawStates) do
			local overlayName = node.overlay and node.overlay[state .. (node.ascendancyName and "Ascend" or "") .. (node.isBlighted and "Blighted" or "")]
			if isAbyssConquered(node) and (node.type == "Notable" or node.type == "Keystone") then
				local frameType = node.ascendancyName and "Ascendancy" or node.type
				local frameState = state == "alloc" and "Allocated" or state == "path" and "CanAllocate" or "Unallocated"
				local abyssOverlay = "Abyss" .. frameType .. "Frame" .. frameState
				if assets[abyssOverlay] then
					overlayName = abyssOverlay
				end
			end
			if node.ascendancyName then
				local prefix = node.bloodlineOverlayPrefix or (tree.bloodlineSpritePrefixes and tree.bloodlineSpritePrefixes[node.ascendancyName])
				if prefix and overlayName then
					overlayName = prefix .. overlayName
				end
			end
			result.overlay[i] = overlayName and a(assets[overlayName])
		end
	end
	return result
end

-- The tree assets the host draws by name: the background, the class backgrounds, the group
-- backgrounds, the connectors in each state, the jewel socket overlays; and the view's own
-- rings (PassiveTreeView.lua:29-60) under their field names
local namedAssets = { "^Background", "^PSGroupBackground", "^GroupBackground", "LineConnector", "^Orbit%d",
	"^JewelSocketActive", "^CharmSocketActive" }
local viewImages = {
	ring = "Assets/ring.png", highlightRing = "Assets/small_ring.png",
	jewelShadedOuterRing = "Assets/ShadedOuterRing.png", jewelShadedOuterRingFlipped = "Assets/ShadedOuterRingFlipped.png",
	jewelShadedInnerRing = "Assets/ShadedInnerRing.png", jewelShadedInnerRingFlipped = "Assets/ShadedInnerRingFlipped.png",
	eternal1 = "TreeData/PassiveSkillScreenEternalEmpireJewelCircle1.png", eternal2 = "TreeData/PassiveSkillScreenEternalEmpireJewelCircle2.png",
	karui1 = "TreeData/PassiveSkillScreenKaruiJewelCircle1.png", karui2 = "TreeData/PassiveSkillScreenKaruiJewelCircle2.png",
	maraketh1 = "TreeData/PassiveSkillScreenMarakethJewelCircle1.png", maraketh2 = "TreeData/PassiveSkillScreenMarakethJewelCircle2.png",
	templar1 = "TreeData/PassiveSkillScreenTemplarJewelCircle1.png", templar2 = "TreeData/PassiveSkillScreenTemplarJewelCircle2.png",
	vaal1 = "TreeData/PassiveSkillScreenVaalJewelCircle1.png", vaal2 = "TreeData/PassiveSkillScreenVaalJewelCircle2.png",
	kalguur1 = "TreeData/PassiveSkillScreenKalguuranJewelCircle1.png", kalguur2 = "TreeData/PassiveSkillScreenKalguuranJewelCircle2.png",
}

local function named_assets(tree)
	local list = {}
	for name, data in pairs(tree.assets) do
		for _, pattern in ipairs(namedAssets) do
			if name:match(pattern) then
				list[#list + 1] = { name = name, art = art(tree, data) }
				break
			end
		end
	end
	for name, path in pairs(viewImages) do
		if not sheetIndex[path] then
			table.insert(sheets, path)
			sheetIndex[path] = #sheets
		end
		list[#list + 1] = { name = name, art = { sheet = sheetIndex[path] } }
	end
	table.sort(list, function(a, b) return a.name < b.name end)
	return list
end

-- A group's background art (renderGroup, PassiveTreeView.lua:568-641): an ascendancy's
-- emblem at its start group, else the largest orbit's background, a half one mirrored below
local function group_art(tree, group, isExpansion)
	local x, y = group.x, group.y
	if group.ascendancyName then
		if not group.isAscendancyStart then
			return nil
		end
		local entry = { x = x, y = y, art = art(tree, tree.assets["Classes" .. group.ascendancyName]), ascendancy = group.ascendancyName }
		if tree.classes[1].ascendancies then
			local data, alternate
			for _, class in ipairs(tree.classes) do
				for _, ascendancy in ipairs(class.ascendancies) do
					if ascendancy.id == group.ascendancyName then
						data = ascendancy
						break
					end
				end
				if data then break end
			end
			if not data and tree.alternate_ascendancies then
				for _, ascendancy in pairs(tree.alternate_ascendancies) do
					if ascendancy.id == group.ascendancyName then
						data, alternate = ascendancy, true
						break
					end
				end
			end
			if data and data.flavourTextRect then
				local rect = data.flavourTextRect
				entry.flavour = {
					text = data.flavourText,
					colour = data.flavourTextColour,
					x = x + rect.x - (alternate and 744 or 650),
					y = y + rect.y - (alternate and 706 or 650),
				}
			end
		end
		return entry.art and entry or nil
	end
	local name, half
	if group.oo[3] then
		name, half = isExpansion and "GroupBackgroundLargeHalfAlt" or "PSGroupBackground3", true
	elseif group.oo[2] then
		name = isExpansion and "GroupBackgroundMediumAlt" or "PSGroupBackground2"
	elseif group.oo[1] then
		name = isExpansion and "GroupBackgroundSmallAlt" or "PSGroupBackground1"
	end
	local found = name and art(tree, tree.assets[name])
	return found and { x = x, y = y, art = found, half = half } or nil
end

-- The groups' art of the main tree, then of the spec's cluster jewels (PassiveTreeView.lua:643-650)
local function groups_art(spec)
	local tree = spec.tree
	local list = {}
	local ids = {}
	for id, group in pairs(tree.groups) do
		if not group.isProxy then
			ids[#ids + 1] = id
		end
	end
	table.sort(ids, function(a, b) return tostring(a) < tostring(b) end)
	for _, id in ipairs(ids) do
		list[#list + 1] = group_art(tree, tree.groups[id])
	end
	local subIds = {}
	for id in pairs(spec.subGraphs) do
		subIds[#subIds + 1] = id
	end
	table.sort(subIds)
	for _, id in ipairs(subIds) do
		list[#list + 1] = group_art(tree, spec.subGraphs[id].group, true)
	end
	return list
end

-- PoB's class background art and where it goes (PassiveTreeView.lua:546-564)
local classArt = {
	{ 1, "BackgroundStr", -2750, 1600 }, { 2, "BackgroundDex", 2550, 1600 }, { 3, "BackgroundInt", -250, -2200 },
	{ 4, "BackgroundStrDex", -150, 2350 }, { 5, "BackgroundStrInt", -2100, -1500 }, { 6, "BackgroundDexInt", 2350, -1950 },
}

local function class_art(tree)
	local list = {}
	for _, entry in ipairs(classArt) do
		local found = art(tree, tree.assets[entry[2]])
		if found then
			list[#list + 1] = { class = entry[1], art = found, x = entry[3], y = entry[4] }
		end
	end
	return list
end

-- A connector as the host draws it: its nodes, its art's name (`type` and the state's suffix),
-- where its ends are, and for an orbit's arc the quad PassiveTree:BuildArc made, as offsets from
-- the group's centre per unit of art size (BuildArc scales it by the art's width, which the
-- host knows). A line's quad is the host's to make from its ends (BuildConnector).
local function connector_entry(tree, connector, nodes)
	local n1, n2 = nodes[connector.nodeId1], nodes[connector.nodeId2]
	local entry = {
		ids = { connector.nodeId1, connector.nodeId2 },
		kind = connector.type,
		ascendancy = connector.ascendancyName,
		ends = { { n1.x, n1.y }, { n2.x, n2.y } },
		-- the game's abyss art when both ends are conquered (PassiveTreeView.lua:715-718)
		abyss = isAbyssConquered(n1) and isAbyssConquered(n2),
	}
	if connector.type ~= "LineConnector" then
		local vert = connector.vert.Normal
		local normal = tree.assets[connector.type .. "Normal"]
		local size = (normal and normal.width or 1) * 2 * 1.33
		entry.centre = { vert[1], vert[2] }
		entry.quad = {
			{ (vert[3] - vert[1]) / size, (vert[4] - vert[2]) / size },
			{ (vert[5] - vert[1]) / size, (vert[6] - vert[2]) / size },
			{ (vert[7] - vert[1]) / size, (vert[8] - vert[2]) / size },
		}
		entry.p = connector.c[12]
	end
	return entry
end

-- The connectors PoB draws for the spec: the tree's, then its cluster jewels' (PassiveTreeView.lua:739-746)
local function connectors_of(spec)
	local tree = spec.tree
	local list = {}
	for _, connector in ipairs(tree.connectors) do
		list[#list + 1] = connector_entry(tree, connector, spec.nodes)
	end
	local subIds = {}
	for id in pairs(spec.subGraphs) do
		subIds[#subIds + 1] = id
	end
	table.sort(subIds)
	for _, id in ipairs(subIds) do
		for _, connector in ipairs(spec.subGraphs[id].connectors) do
			list[#list + 1] = connector_entry(tree, connector, spec.nodes)
		end
	end
	return list
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
	local spec = pob.build.spec
	local parts = { spec.treeVersion }
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
			-- the art PassiveTreeView:Draw picks for the node, beyond its icons
			isAbyssConquered(node) and "abyss" or "",
			node.isBlighted and "blighted" or "",
			spec.hashOverrides and spec.hashOverrides[node.id] and spec.hashOverrides[node.id].icon or "",
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

-- Whether a socket's radius can hold the node, as PassiveTree lists the nodes in each socket's
-- radius (PassiveTree.lua:645-648): a tree node, not a cluster jewel's
local function in_socket_radius(tree, id)
	local node = tree.nodes[id]
	return node ~= nil and not node.isBlighted and node.group ~= nil and not node.isProxy
		and not node.group.isProxy and not node.isMastery or nil
end

-- Geometry of the active spec's tree, for drawing it. Positions are PoB's own, in tree units.
function glue.tree_layout()
	sheets, sheetIndex = {}, {}
	local spec = pob.build.spec
	local tree = spec.tree
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
			art = nodeArt(node, spec),
			-- a node a hovered socket's radius colours (PassiveTree.lua:645-648)
			in_radius = in_socket_radius(tree, node.id),
			-- a socket whose hover colours the nodes in its radius (PassiveTreeView.lua:950-951)
			radius_socket = tree.sockets[node.id] ~= nil and tree.sockets[node.id].nodesInRadius ~= nil or nil,
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
	local radii = {}
	for i, radius in ipairs(pob.build.data.jewelRadius) do
		radii[i] = { inner = radius.inner, outer = radius.outer, colour = radius.col }
	end
	return {
		version = pob.build.spec.treeVersion,
		digest = layout_digest(placed),
		nodes = nodes,
		edges = edges,
		connectors = connectors_of(spec),
		groups = groups_art(spec),
		class_art = class_art(tree),
		assets = named_assets(tree),
		size = tree.size,
		jewel_radii = radii,
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

-- A compare-only node's allocated base and overlay, as PoB draws them tinted green
-- (PassiveTreeView.lua:1108-1140), from the files the layout sent
local function compare_only_art(build, compareSpec, node)
	local tree = build.spec.tree
	local viewer = build.treeTab.viewer
	local base, overlay
	sheetsFrozen = true
	local ok, err = pcall(function()
		if node.type == "Socket" then
			base = art(tree, tree.assets[node.overlay and node.overlay["alloc" .. (node.expansionJewel and "Alt" or "")] or "JewelSocketActiveBlue"])
			local saved = viewer.compareSpec
			viewer.compareSpec = compareSpec
			local jewelOk, jewel = pcall(viewer.GetCompareJewel, viewer, node.id)
			viewer.compareSpec = saved
			if jewelOk and jewel then
				local name = viewer:GetJewelSocketOverlay(jewel, node.expansionJewel)
				overlay = name and art(tree, tree.assets[name])
			end
		elseif node.type == "Mastery" then
			if node.masterySprites and node.masterySprites.activeIcon then
				base = art(compareSpec.tree, node.masterySprites.activeIcon.masteryActiveSelected)
			elseif node.sprites then
				base = art(compareSpec.tree, node.sprites.mastery)
			end
		else
			if node.sprites then
				base = art(compareSpec.tree, node.sprites[node.type:lower() .. "Active"])
			end
			if node.overlay then
				local name = node.overlay["alloc" .. (node.ascendancyName and "Ascend" or "") .. (node.isBlighted and "Blighted" or "")]
				overlay = name and art(tree, tree.assets[name])
			end
		end
	end)
	sheetsFrozen = false
	if not ok then
		return nil
	end
	return { base = base, overlay = overlay }
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
					mastery_effects = glue.node_mastery_effects(node),
					-- PoB draws its allocated art, tinted green (PassiveTreeView.lua:1108-1140),
					-- from the sheets the layout sent
					art = sheetIndex and compare_only_art(build, compareSpec, node) or nil,
				}
			end
		end
		table.sort(list, function(a, b) return a.id < b.id end)
	end
	return list
end

-- Ids of the nodes `test` picks, sorted
local function ids_where(nodes, test)
	local ids = {}
	for id, node in pairs(nodes) do
		if test(id, node) then
			ids[#ids + 1] = id
		end
	end
	table.sort(ids)
	return ids
end

-- What the drawn tree's art depends on beyond the spec's allocation (PassiveTreeView.lua:805-813,
-- 840-845): nodes the build grants, nodes the compared spec allocates, Split Personality's path,
-- and the overlay of each socket's jewel
local function draw_state(build)
	local spec = build.spec
	local compareSpec = viewed_compare_spec(build)
	local granted = build.calcsTab.mainEnv and build.calcsTab.mainEnv.grantedPassives or {}
	local split = spec.splitPersonalityPath or {}
	local viewer = build.treeTab.viewer
	local overlays = {}
	for _, id in ipairs(ids_where(spec.nodes, function(_, node) return node.type == "Socket" end)) do
		local _, jewel = build.itemsTab:GetSocketAndJewelForNodeID(id)
		local name = jewel and viewer:GetJewelSocketOverlay(jewel, spec.nodes[id].expansionJewel)
		if name then
			overlays[#overlays + 1] = { node = id, overlay = name }
		end
	end
	local variable = {}
	for _, id in ipairs(ids_where(spec.nodes, function(_, node) return node.type == "Socket" end)) do
		local _, jewel = build.itemsTab:GetSocketAndJewelForNodeID(id)
		if jewel and jewel.jewelRadiusLabel == "Variable" then
			variable[#variable + 1] = id
		end
	end
	return {
		comparing = compareSpec ~= nil,
		-- nodes the compared spec doesn't have: their connectors keep their own state (PassiveTreeView.lua:689-694)
		compare_missing = compareSpec and ids_where(spec.nodes, function(id) return compareSpec.nodes[id] == nil end) or {},
		variable_sockets = variable,
		granted = ids_where(spec.nodes, function(id) return granted[id] and true end),
		compare_allocated = compareSpec and ids_where(spec.nodes, function(id)
			return compareSpec.nodes[id] ~= nil and compareSpec.nodes[id].alloc
		end) or {},
		split_path = ids_where(spec.nodes, function(id) return split[id] and true end),
		-- nodes an Intuitive Leap-like jewel reaches: hovering one shows no path (PassiveTreeView.lua:367-371)
		leap = ids_where(spec.nodes, function(_, node)
			return node.intuitiveLeapLikesAffecting ~= nil and #node.intuitiveLeapLikesAffecting > 0
		end),
		socket_overlays = overlays,
	}
end

local jewelRingArt = {
	{ "^Brutal Restraint", "maraketh1", "maraketh2" }, { "^Elegant Hubris", "eternal1", "eternal2" },
	{ "^Glorious Vanity", "vaal1", "vaal2" }, { "^Lethal Pride", "karui1", "karui2" },
	{ "^Militant Faith", "templar1", "templar2" }, { "^Heroic Tragedy", "kalguur1", "kalguur2" },
}

-- The rings PoB draws for a socketed jewel (drawJewelRadius, PassiveTreeView.lua:1156-1196):
-- each a view image by its field name, its size across in tree units, where it is, how fast it
-- turns (`DrawImageRotated`'s angle) and its tint
local function jewel_ring_images(build, jewel, x, y, tint, list)
	if isAbyssConquered(jewel.jewelData) then
		return
	end
	local radius = build.data.jewelRadius[jewel.jewelRadiusIndex]
	if not radius then
		return
	end
	local outer, inner = radius.outer * 2, radius.inner * 1.06 * 2
	local function add(image, size, spin, at)
		list[#list + 1] = { image = image, size = size, spin = spin, x = at and at.x or x, y = at and at.y or y, tint = tint }
	end
	if jewel.title:match("Impossible Escape") then
		for name in pairs(jewel.jewelData.impossibleEscapeKeystones or {}) do
			local keystone = build.spec.tree.keystoneMap[name]
			if keystone and keystone.x and keystone.y then
				add("jewelShadedOuterRing", outer, -0.8, keystone)
				add("jewelShadedOuterRingFlipped", outer, 1, keystone)
				add("jewelShadedInnerRing", 300, -1.2, keystone)
				add("jewelShadedInnerRingFlipped", 300, 1.0, keystone)
			end
		end
		return
	end
	for _, entry in ipairs(jewelRingArt) do
		if jewel.title:match(entry[1]) then
			add(entry[2], outer, -0.7)
			add(entry[3], outer, 0.7)
			return
		end
	end
	add("jewelShadedOuterRing", outer, -0.7)
	add("jewelShadedOuterRingFlipped", outer, 0.7)
	add("jewelShadedInnerRing", inner, -0.7)
	add("jewelShadedInnerRingFlipped", inner, 0.7)
end

local ringNeutral, ringPrimary, ringCompare = { 1, 1, 1, 0.7 }, { 1, 0, 0, 0.7 }, { 0, 1, 0, 0.7 }

-- The sockets that draw radius rings (PassiveTreeView.lua:1198-1248): whether a hover draws the
-- Thread of Hope annuli (`variable`), and the rings of the socket's jewels
local function socket_rings(build)
	local spec = build.spec
	local viewer = build.treeTab.viewer
	local compareSpec = viewed_compare_spec(build)
	local saved = viewer.compareSpec
	viewer.compareSpec = compareSpec
	local list = {}
	local ok, err = pcall(function()
		local ids = {}
		for id in pairs(spec.tree.sockets) do
			ids[#ids + 1] = id
		end
		table.sort(ids)
		for _, id in ipairs(ids) do
			local node = spec.nodes[id]
			if node and node.name ~= "Charm Socket" and (not node.expansionJewel or node.expansionJewel.size == 2) then
				local _, jewel = build.itemsTab:GetSocketAndJewelForNodeID(id)
				local compareNode = compareSpec and compareSpec.nodes[id]
				local cJewel = compareSpec and viewer:GetCompareJewel(id) or nil
				local effective = jewel or cJewel
				local entry = { node = id, variable = effective and effective.jewelRadiusLabel == "Variable" or false, rings = {} }
				if node.alloc or (compareNode and compareNode.alloc) then
					local same
					if not jewel or not cJewel then
						same = jewel == cJewel
					else
						same = jewel:BuildRaw() == cJewel:BuildRaw()
					end
					if jewel and jewel.jewelRadiusIndex then
						jewel_ring_images(build, jewel, node.x, node.y, (not compareSpec or same) and ringNeutral or ringPrimary, entry.rings)
					end
					if cJewel and cJewel.jewelRadiusIndex and not same then
						jewel_ring_images(build, cJewel, node.x, node.y, ringCompare, entry.rings)
					end
				end
				list[#list + 1] = entry
			end
		end
	end)
	viewer.compareSpec = saved
	if not ok then
		error(err, 0)
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
		draw = draw_state(build),
		socket_rings = socket_rings(build),
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
