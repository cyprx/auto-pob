-- Helpers for the tree recipes. Runs in upstream PoB after the base build is loaded.
-- Every `act.*` performs the action through PoB's own steps (cited by file and line),
-- records it in `actions` for the gate to replay as our commands, then runs a frame.
-- Nothing here may call our glue.

actions = {}
act = {}
pick = {}

local function frame()
	runCallback("OnFrame")
end

local function classByName(name)
	for id, class in pairs(build.spec.tree.classes) do
		if class.name == name then return id end
	end
	error("no class " .. name)
end

-- Presses the button with this label on the newest popup (or its `confirm` control).
local function pressPopup(label, key)
	local popup = main.popups[1]
	assert(popup, "no popup is open")
	for k, control in pairs(popup.controls) do
		if control.onClick and ((label and control.label == label) or (key and k == key)) then
			control.onClick()
			return
		end
	end
	error("no button " .. tostring(label or key))
end

-- A left click on an unallocated node (PassiveTreeView.lua:493-496)
function act.alloc(node)
	build.spec:AllocNode(node)
	build.spec:AddUndoState()
	build.buildFlag = true
	table.insert(actions, { alloc = node.id })
	frame()
end

-- A left click on an unallocated node of another ascendancy of the same class: switches to it
-- first (PassiveTreeView.lua:421-446), then allocates as usual.
function act.alloc_ascendancy(node)
	local spec = build.spec
	for ascendClassId, ascendClass in pairs(spec.curClass.classes) do
		if ascendClass.id == node.ascendancyName then
			spec:SelectAscendClass(ascendClassId)
			spec:AddUndoState()
			spec:SetWindowTitleWithBuildClass()
			build.buildFlag = true
		end
	end
	act.alloc(spec.nodes[node.id])
end

-- A left click on an allocated node (PassiveTreeView.lua:384-388)
function act.dealloc(node)
	build.spec:DeallocNode(node)
	build.spec:AddUndoState()
	build.buildFlag = true
	table.insert(actions, { dealloc = node.id })
	frame()
end

-- The class dropdown (Build.lua:255); a confirm popup is answered "Connect Path" or "Continue"
function act.class(name, connect)
	local id = classByName(name)
	local drop = build.controls.classDrop
	for _, entry in ipairs(drop.list) do
		if entry.classId == id then
			local popups = #main.popups
			drop.selFunc(nil, entry)
			if #main.popups > popups then
				if connect then pressPopup("Connect Path") else pressPopup(nil, "confirm") end
			end
		end
	end
	assert(build.spec.curClassId == id, "class did not change")
	table.insert(actions, { class = id, connect = connect })
	frame()
end

-- The ascendancy dropdown (Build.lua:279-283)
function act.ascend(id)
	build.controls.ascendDrop.selFunc(nil, { ascendClassId = id })
	table.insert(actions, { ascend = id })
	frame()
end

-- The Reset Tree button and its popup (TreeTab.lua:133-138)
function act.reset()
	build.treeTab.controls.reset.onClick()
	pressPopup(nil, "reset")
	table.insert(actions, { reset = true })
	frame()
end

-- The mastery popup's save (TreeTab.lua:992-1007)
function act.mastery(node, effect)
	build.treeTab:SaveMasteryPopup(node, { selValue = { id = effect } })
	table.insert(actions, { mastery = node.id, effect = effect })
	frame()
end

-- Ctrl+Z / Ctrl+Y on the tree tab (TreeTab.lua:358-366)
function act.undo()
	build.spec:Undo()
	build.buildFlag = true
	table.insert(actions, { undo = true })
	frame()
end

function act.redo()
	build.spec:Redo()
	build.buildFlag = true
	table.insert(actions, { redo = true })
	frame()
end

-- Opens the Manage Passive Trees popup (TreeTab.lua:630), calls `fn(list)` with its
-- PassiveSpecListControl and closes what is left open.
local function withSpecList(fn)
	local depth = #main.popups
	build.treeTab:OpenSpecManagePopup()
	local list
	for _, control in pairs(main.popups[1].controls) do
		if control.controls and control.controls.new and control.OnSelDelete then list = control end
	end
	assert(list, "no tree list")
	fn(list)
	while #main.popups > depth do main:ClosePopup() end
end

-- Types the name into the rename popup on top and presses Save (PassiveSpecListControl.lua:37-47)
local function saveName(title)
	main.popups[1].controls.edit.buf = title
	pressPopup(nil, "save")
end

-- Indices in the records are 0-based, as in our commands.

-- The New button and the name popup (PassiveSpecListControl.lua:31-58)
function act.new_spec(title)
	withSpecList(function(list)
		list.controls.new.onClick()
		saveName(title)
	end)
	table.insert(actions, { new_spec = title })
	frame()
end

-- The tree dropdown (TreeTab.lua:39-43)
function act.select_spec(index)
	build.treeTab.controls.specSelect.selFunc(index + 1, build.treeTab.specList[index + 1])
	table.insert(actions, { select_spec = index })
	frame()
end

-- The Rename button and the name popup (PassiveSpecListControl.lua:26-30, 37-47)
function act.rename_spec(index, title)
	withSpecList(function(list)
		list:RenameSpec(build.treeTab.specList[index + 1], "Rename Tree")
		saveName(title)
	end)
	table.insert(actions, { rename_spec = index, title = title })
	frame()
end

-- The Delete button's confirmation (PassiveSpecListControl.lua:87-101)
function act.delete_spec(index)
	withSpecList(function(list)
		list:OnSelDelete(index + 1, build.treeTab.specList[index + 1])
		pressPopup(nil, "confirm")
	end)
	table.insert(actions, { delete_spec = index })
	frame()
end

-- The Compare checkbox and dropdown (TreeTab.lua:106-122); false switches comparing off
function act.compare(index)
	local controls = build.treeTab.controls
	if index == false then
		controls.compareCheck.state = false
		controls.compareCheck.changeFunc(false)
	else
		controls.compareCheck.state = true
		controls.compareCheck.changeFunc(true)
		controls.compareSelect.selFunc(index + 1, build.treeTab.specList[index + 1])
	end
	table.insert(actions, { compare = index })
	frame()
end

-- The version dropdown's popup (TreeTab.lua:166, 649-666)
function act.convert(version, keepCopy)
	build.treeTab:OpenVersionConvertPopup(version, true)
	pressPopup(keepCopy and "Copy + Convert" or "Convert")
	table.insert(actions, { convert = version, keep_copy = keepCopy })
	frame()
end

-- The Convert all popup (TreeTab.lua:668-681)
function act.convert_all(version)
	build.treeTab:OpenVersionConvertAllPopup(version)
	pressPopup("Convert")
	table.insert(actions, { convert_all = version })
	frame()
end

-- The Import Tree popup (TreeTab.lua:682-811)
function act.import(url, title)
	build.treeTab:OpenImportPopup()
	local controls = main.popups[1].controls
	controls.name.buf = title
	controls.edit.buf = url
	controls.import.onClick()
	assert(#main.popups == 0, "the import popup stayed open: " .. tostring(controls.msg.label))
	table.insert(actions, { import = url, title = title })
	frame()
end

-- The link OpenExportPopup shows (TreeTab.lua:813)
function exportLink()
	return build.spec:EncodeURL(treeVersions[build.spec.treeVersion].url)
end

-- What the tree view colours when comparing: nodes of the active spec whose compare colour
-- isn't the default (TreeTab.lua:445, PassiveTreeView.lua:808, 936). Sorted by node.
function compareColours()
	local tab = build.treeTab
	local compareSpec = tab.isComparing and tab.specList[tab.activeCompareSpec] or nil
	tab.viewer.compareSpec = compareSpec
	local colours = {}
	if compareSpec then
		for id, node in pairs(build.spec.nodes) do
			local r, g, b = tab.viewer:GetCompareNodeColor(node, compareSpec.nodes[id], build.spec, build, nil)
			if r then colours[#colours + 1] = { node = id, rgb = { r, g, b } } end
		end
		table.sort(colours, function(a, b) return a.node < b.node end)
	end
	return colours
end

-- The tree state right now, recorded for the gate to compare with ours at this point.
function checkpoint()
	local allocated, titles = {}, {}
	for id in pairs(build.spec.allocNodes) do allocated[#allocated + 1] = id end
	table.sort(allocated)
	for i, spec in ipairs(build.treeTab.specList) do titles[i] = spec.title or "Default" end
	table.insert(actions, { check = {
		allocated = allocated, spec_titles = titles,
		active_spec = build.treeTab.activeSpec - 1, compare = compareColours(),
	} })
end

-- The best node by (pathDist, id) among the unallocated nodes that `ok` accepts.
local function nearest(ok)
	local best
	for _, node in pairs(build.spec.nodes) do
		if node.path and not node.alloc and ok(node) then
			if not best or node.pathDist < best.pathDist
				or (node.pathDist == best.pathDist and node.id < best.id) then
				best = node
			end
		end
	end
	assert(best, "no node matches")
	return best
end

-- The nearest unallocated main-tree Notable at least `minDist` points away.
function pick.notable(minDist)
	return nearest(function(node)
		return node.type == "Notable" and not node.ascendancyName and not node.isProxy
			and node.pathDist >= minDist
	end)
end

-- The nearest reachable Mastery that offers effects.
function pick.mastery()
	return nearest(function(node)
		return node.type == "Mastery" and node.masteryEffects and #node.masteryEffects > 1
	end)
end

-- A main-tree node PoB found no path to (some Notables and a Keystone are cut off).
function pick.unreachable()
	local best
	for _, node in pairs(build.spec.nodes) do
		if not node.path and not node.alloc and not node.ascendancyName and node.type ~= "Mastery"
			and node.type ~= "ClassStart" and not node.isProxy and node.x and (not best or node.id < best.id) then
			best = node
		end
	end
	assert(best, "every node is reachable")
	return best
end

-- The node in the middle of the path to `node`; take it before allocating.
function pick.path_middle(node)
	local path = node.path
	return path[math.ceil(#path / 2)]
end

-- The effects OpenMasteryPopup would list for `node`, sorted by effect id (TreeTab.lua:1009).
function pick.mastery_effects(node)
	local effects = {}
	for _, effect in pairs(node.masteryEffects) do
		local assigned = isValueInTable(build.spec.masterySelections, effect.effect)
		if not assigned or assigned == node.id then
			effects[#effects + 1] = effect.effect
		end
	end
	table.sort(effects)
	return effects
end

-- The lowest-id non-start node of the lowest-id ascendancy of the current class that isn't
-- selected (for a click that switches ascendancy: PassiveTreeView.lua:421).
function pick.ascendancy_node()
	local spec = build.spec
	local ascendId
	for id, ascendClass in pairs(spec.curClass.classes) do
		if id ~= 0 and id ~= spec.curAscendClassId and (not ascendId or id < ascendId) then
			ascendId = id
		end
	end
	assert(ascendId, "no other ascendancy")
	local name = spec.curClass.classes[ascendId].id
	local best
	for _, node in pairs(spec.nodes) do
		if node.ascendancyName == name and node.type ~= "AscendClassStart"
			and (not best or node.id < best.id) then
			best = node
		end
	end
	assert(best, "no node in " .. name)
	return best
end

-- The name of the class with the lowest id, other than the current one, that the tree is not
-- connected to.
function pick.unconnected_class()
	local best
	for id in pairs(build.spec.tree.classes) do
		if id ~= build.spec.curClassId and not build.spec:IsClassConnected(id) and (not best or id < best) then
			best = id
		end
	end
	assert(best, "every class is connected")
	return build.spec.tree.classes[best].name
end
