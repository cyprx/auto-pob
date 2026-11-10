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
