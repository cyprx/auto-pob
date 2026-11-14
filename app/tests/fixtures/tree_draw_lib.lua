-- Records what PassiveTreeView:Draw draws, for the gate's comparison with our canvas.
-- `recordTreeDraw(viewer, build)` draws the tree once with the drawing functions replaced and
-- returns every image drawn, in order:
--   { kind = "image" | "quad", file, rect (pixels, or nil for the whole file), x, y (tree
--     units: the image's or the quad's centre), colour = { r, g, b, a }, layer,
--     connector = { id1, id2 } for a connector's quad, flipped = true for a lower half }
-- The headless wrapper's images are 1x1, so sizes mean nothing: a sprite's texture
-- coordinates come out in pixels, and a drawn image is about 2.7 tree units across.

local function sheetPath(tree, filename)
	local file = filename:gsub("%?%x+$", ""):gsub(".*/", "")
	return "TreeData/" .. tree.treeVersion .. "/" .. file
end

-- Every image handle the tree view can draw, to its file
local function handleFiles(viewer, tree)
	local files = {}
	-- the view's own images (PassiveTreeView.lua:29-60)
	local own = {
		ring = "Assets/ring.png", highlightRing = "Assets/small_ring.png",
		jewelShadedOuterRing = "Assets/ShadedOuterRing.png",
		jewelShadedOuterRingFlipped = "Assets/ShadedOuterRingFlipped.png",
		jewelShadedInnerRing = "Assets/ShadedInnerRing.png",
		jewelShadedInnerRingFlipped = "Assets/ShadedInnerRingFlipped.png",
		eternal1 = "TreeData/PassiveSkillScreenEternalEmpireJewelCircle1.png",
		eternal2 = "TreeData/PassiveSkillScreenEternalEmpireJewelCircle2.png",
		karui1 = "TreeData/PassiveSkillScreenKaruiJewelCircle1.png",
		karui2 = "TreeData/PassiveSkillScreenKaruiJewelCircle2.png",
		maraketh1 = "TreeData/PassiveSkillScreenMarakethJewelCircle1.png",
		maraketh2 = "TreeData/PassiveSkillScreenMarakethJewelCircle2.png",
		templar1 = "TreeData/PassiveSkillScreenTemplarJewelCircle1.png",
		templar2 = "TreeData/PassiveSkillScreenTemplarJewelCircle2.png",
		vaal1 = "TreeData/PassiveSkillScreenVaalJewelCircle1.png",
		vaal2 = "TreeData/PassiveSkillScreenVaalJewelCircle2.png",
		kalguur1 = "TreeData/PassiveSkillScreenKalguuranJewelCircle1.png",
		kalguur2 = "TreeData/PassiveSkillScreenKalguuranJewelCircle2.png",
	}
	for field, path in pairs(own) do
		if viewer[field] then
			files[viewer[field]] = path
		end
	end
	-- sprite sheets: each sprite holds its sheet's handle (PassiveTree.lua:284-305)
	for name, set in pairs(tree.spriteMap) do
		for spriteType, sprite in pairs(set) do
			if sprite.handle and not files[sprite.handle] then
				local data = tree.skillSprites[spriteType]
				if data then
					if tree.imageZoomLevels then
						data = treeVersions[tree.treeVersion].num >= 3.19 and (data[0.3835] or data[1]) or data[#data]
					end
					files[sprite.handle] = sheetPath(tree, data.filename)
				end
			end
		end
	end
	-- the legion sheets (PassiveTree.lua:366-412)
	local legion = LoadModule("TreeData/legion/tree-legion.lua")
	for _, data in ipairs(legion.treeAssets or {}) do
		for name in pairs(data.coords) do
			local asset = tree.assets[name]
			if asset and asset.handle then
				files[asset.handle] = "TreeData/legion/" .. data.filename
			end
		end
	end
	for spriteType, data in pairs(legion) do
		if spriteType ~= "treeAssets" then
			local maxZoom = data[#data]
			for name in pairs(maxZoom.coords) do
				local sprite = tree.spriteMap[name] and tree.spriteMap[name][spriteType]
				if sprite and sprite.handle then
					files[sprite.handle] = "TreeData/legion/" .. maxZoom.filename
				end
			end
		end
	end
	-- whole files (PassiveTree.lua:238-241, LoadImage)
	for name, data in pairs(tree.assets) do
		if data.handle and not files[data.handle] then
			local file = io.open("TreeData/" .. name .. ".png", "r")
			if file then
				file:close()
				files[data.handle] = "TreeData/" .. name .. ".png"
			else
				files[data.handle] = "TreeData/" .. tree.treeVersion .. "/" .. name .. ".png"
			end
		end
	end
	return files
end

-- Every connector Draw can draw, by the table its quad's coordinates are written to
local function connectorsOf(viewer, build)
	local list = {}
	local function add(connectors)
		for _, connector in pairs(connectors) do
			list[#list + 1] = connector
		end
	end
	add(build.spec.tree.connectors)
	for _, subGraph in pairs(build.spec.subGraphs) do
		add(subGraph.connectors)
	end
	if viewer.compareSpec then
		for _, subGraph in pairs(viewer.compareSpec.subGraphs) do
			add(subGraph.connectors)
		end
	end
	return list
end

-- `hover`: a node id to put the cursor on; the second result is the node PoB found under it
function recordTreeDraw(viewer, build, hover)
	local tree = build.spec.tree
	local files = handleFiles(viewer, tree)
	local connectors = connectorsOf(viewer, build)
	-- a square view four times the tree's size across at a zoom of 0.25, so a screen point is a
	-- tree point plus `half` and every node is well inside; the cursor (0, 0) is left of it, so
	-- nothing is hovered
	local half = tree.size * 2
	local viewPort = { x = 10, y = 0, width = tree.size * 4, height = tree.size * 4 }
	local saved = { viewer.zoomLevel, viewer.zoom, viewer.zoomX, viewer.zoomY }
	viewer.zoomLevel, viewer.zoom, viewer.zoomX, viewer.zoomY = 0, 0.25, 0, 0
	local function toTree(x, y)
		return x - 10 - half, y - half
	end

	local colour, layer = { 1, 1, 1, 1 }, 0
	local list = {}
	local real = { SetDrawColor = SetDrawColor, SetDrawLayer = SetDrawLayer, DrawImage = DrawImage,
		DrawImageQuad = DrawImageQuad, DrawString = DrawString, GetCursorPos = GetCursorPos }
	if hover then
		local node = build.spec.nodes[hover]
		function GetCursorPos()
			return node.x + 10 + half, node.y + half
		end
	end
	function SetDrawColor(r, g, b, a)
		if type(r) == "string" then
			-- a colour code: ^xRRGGBB or ^N
			local hex = r:match("^%^x(%x%x%x%x%x%x)")
			if hex then
				colour = { tonumber(hex:sub(1, 2), 16) / 255, tonumber(hex:sub(3, 4), 16) / 255,
					tonumber(hex:sub(5, 6), 16) / 255, 1 }
			else
				colour = { r }
			end
		else
			colour = { r, g, b, a or 1 }
		end
	end
	function SetDrawLayer(_, sub)
		layer = sub or layer
	end
	function DrawImage(handle, left, top, width, height, s1, t1, s2, t2)
		if not handle then
			return
		end
		local x, y = toTree(left + width / 2, top + height / 2)
		list[#list + 1] = {
			kind = "image", file = files[handle] or "?", x = x, y = y,
			rect = s1 and s2 and { s1, t1, s2, t2 } or nil,
			flipped = s1 == 0 and t1 == 1 and s2 == 1 and t2 == 0 or nil,
			colour = { colour[1], colour[2], colour[3], colour[4] }, layer = layer,
		}
	end
	function DrawImageQuad(handle, x1, y1, x2, y2, x3, y3, x4, y4)
		if not handle then
			return
		end
		local entry = {
			kind = "quad", file = files[handle] or "?",
			colour = { colour[1], colour[2], colour[3], colour[4] }, layer = layer,
		}
		entry.x, entry.y = toTree((x1 + x2 + x3 + x4) / 4, (y1 + y2 + y3 + y4) / 4)
		for _, connector in ipairs(connectors) do
			local c = connector.c
			-- arcs of two orbits at the same angles have the same quad with 1x1 images: the
			-- art tells them apart
			if c and entry.file:find(connector.type, 1, true) and c[1] == x1 and c[2] == y1 and c[3] == x2 and c[4] == y2 and c[5] == x3
				and c[6] == y3 and c[7] == x4 and c[8] == y4 then
				entry.connector = { connector.nodeId1, connector.nodeId2 }
				break
			end
		end
		list[#list + 1] = entry
	end
	function DrawString() end
	local ok, err = pcall(viewer.Draw, viewer, build, viewPort, {})
	for name, fn in pairs(real) do
		_G[name] = fn
	end
	viewer.zoomLevel, viewer.zoom, viewer.zoomX, viewer.zoomY = saved[1], saved[2], saved[3], saved[4]
	if not ok then
		error(err, 0)
	end
	return list, viewer.hoverNode and viewer.hoverNode.id
end
