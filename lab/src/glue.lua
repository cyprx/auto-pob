-- Glue between the Rust host and Path of Building's headless calc engine.
--
-- Runs after PoB's HeadlessWrapper.lua, which defines the global `build` (the build
-- currently loaded) and the helpers `newBuild()` and `loadBuildFromXML(xml, name)`.
-- Everything the Rust side needs is exposed through the global `lab` table.

lab = {}

-- Recalculate the build after changing it. PoB recalculates during `OnFrame`
-- when `buildFlag` is set.
local function recalc()
	build.buildFlag = true
	runCallback("OnFrame")
end

-- Load a PoB share code (base64url of the zlib-compressed build XML).
function lab.load_code(code)
	code = code:gsub("%s", "")
	local xml = Inflate(common.base64.decode(code:gsub("-", "+"):gsub("_", "/")))
	if not xml or xml == "" then
		error("Not a valid PoB share code")
	end
	loadBuildFromXML(xml, "Imported build")
end

function lab.load_xml(xml)
	loadBuildFromXML(xml, "Imported build")
end

function lab.new_build()
	newBuild()
end

-- Sets the character level. PoB's default "auto" mode derives the level from
-- allocated points and would overwrite it, so it is turned off.
function lab.set_level(level)
	build.characterLevelAutoMode = false
	build.characterLevel = level
	recalc()
end

-- Sets a value on the Configuration tab (e.g. "usePowerCharges" = true).
function lab.set_config(var, value)
	build.configTab.input[var] = value
	build.configTab:BuildModList()
	recalc()
end

-- Returns what the UI shows: class, level, the requested stats and config values.
function lab.summary(statKeys, configVars)
	local output = build.calcsTab.mainOutput
	local stats = {}
	for i, key in ipairs(statKeys) do
		stats[i] = output[key] -- nil if this build doesn't have the stat
	end
	local config = {}
	for i, var in ipairs(configVars) do
		config[i] = build.configTab.input[var]
	end
	local allocated = {}
	for id in pairs(build.spec.allocNodes) do
		table.insert(allocated, id)
	end
	return {
		allocated = allocated,
		class = build.spec.curClassName,
		ascendancy = build.spec.curAscendClassName,
		level = build.characterLevel,
		stats = stats,
		config = config,
	}
end

-- Scores every unallocated passive by how much `metric` (a key of the calc output,
-- e.g. "Life" or "CombinedDPS") would change if it were allocated. Mirrors PoB's own
-- node power: the node alone, and the node together with the path leading to it.
-- Uses PoB's what-if calculator, so the build itself is not modified.
-- `progress(done, total)` is called periodically.
function lab.node_power(metric, maxDistance, progress)
	local calc, base = build.calcsTab:GetMiscCalculator()
	local baseValue = base[metric] or 0

	local candidates = {}
	for _, node in pairs(build.spec.nodes) do
		local distance = node.path and #node.path or 0
		if not node.alloc and not node.ascendancyName and node.modKey ~= ""
			and node.type ~= "Mastery" and distance > 0 and distance <= maxDistance then
			table.insert(candidates, node)
		end
	end

	-- nodes with identical modifiers (e.g. "+10 to Strength") give identical results
	local singleCache = {}
	local results = {}
	for i, node in ipairs(candidates) do
		local single = singleCache[node.modKey]
		if not single then
			single = (calc({ addNodes = { [node] = true } })[metric] or 0) - baseValue
			singleCache[node.modKey] = single
		end

		local distance = #node.path
		local withPath = single
		if distance > 1 then
			local pathNodes = {}
			for _, pathNode in ipairs(node.path) do
				pathNodes[pathNode] = true
			end
			withPath = (calc({ addNodes = pathNodes })[metric] or 0) - baseValue
		end

		results[i] = {
			id = node.id,
			name = node.dn,
			kind = node.type,
			distance = distance,
			gain = single,
			path_gain = withPath,
			per_point = withPath / distance,
		}
		if i % 20 == 0 then
			progress(i, #candidates)
		end
	end
	return results
end

-- Sprite sheets used by lab.tree(), as paths relative to the PoB folder.
local sheets, sheetIndex

-- Looks up a sprite the way PoB's PassiveTree does, returning its sheet and pixel
-- rectangle. PoB's own UVs can't be used: headless images report a 1x1 size.
local function sprite(spriteType, name)
	local tree = build.spec.tree
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

-- Geometry of the main passive tree, for drawing it. Positions are PoB's own,
-- in tree units. Ascendancy trees are left out.
function lab.tree()
	sheets, sheetIndex = {}, {}
	local nodes, edges = {}, {}
	for id, node in pairs(build.spec.nodes) do
		if node.x and not node.ascendancyName then
			local stats = {}
			for i, line in ipairs(node.sd or {}) do
				stats[i] = line
			end
			table.insert(nodes, {
				id = id,
				x = node.x,
				y = node.y,
				kind = node.type,
				name = node.dn or "",
				stats = stats,
				-- nodes on the same orbit of the same group are connected by an arc
				group = node.g,
				orbit = node.o,
				group_x = node.group and node.group.x,
				group_y = node.group and node.group.y,
				art = nodeArt(node),
			})
			for _, other in ipairs(node.linked or {}) do
				-- each connection is listed on both nodes; keep one
				if id < other.id and other.x and not other.ascendancyName then
					table.insert(edges, { id, other.id })
				end
			end
		end
	end
	return { nodes = nodes, edges = edges, sheets = sheets }
end
