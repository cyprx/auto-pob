-- Helpers for the fixture recipes. Runs in upstream PoB after newBuild().

local function classIdByName(name)
	for id, class in pairs(build.spec.tree.classes) do
		if class.name == name then return id end
	end
	error("no class " .. name)
end

-- Allocates `count` nodes next to the class start: cheapest first, ties by node id.
local function allocClassSide(count)
	for _ = 1, count do
		local best
		for id, node in pairs(build.spec.nodes) do
			if node.path and not node.alloc and not node.ascendancyName
				and node.type ~= "Mastery" and node.type ~= "ClassStart"
				and not node.isMultipleChoiceOption and not node.isProxy then
				if not best or node.pathDist < best.pathDist
					or (node.pathDist == best.pathDist and node.id < best.id) then
					best = node
				end
			end
		end
		assert(best, "no node left to allocate")
		build.spec:AllocNode(best, best.path)
	end
end

-- spec: { weapon = "<raw item text>" (optional), class = "Witch", level = 90, gems = { "Essence Drain", ... }, nodes = 20 }
function buildFixture(spec)
	build.spec:SelectClass(classIdByName(spec.class))

	build.characterLevelAutoMode = false
	build.characterLevel = spec.level
	build.controls.characterLevel:SetText(spec.level)
	build.controls.levelScalingButton.label = "Manual"
	build.configTab:BuildModList()

	allocClassSide(spec.nodes)

	local text = ""
	for _, gem in ipairs(spec.gems) do
		text = text .. gem .. " 20/0  1\r\n"
	end
	build.skillsTab:PasteSocketGroup(text)

	-- the first group is the main skill
	build.mainSocketGroup = 1

	if spec.weapon then
		local item = new("Item", spec.weapon)
		build.itemsTab:AddItem(item, false)
		build.itemsTab:PopulateSlots()
	end

	build.modFlag = true
	build.buildFlag = true
	runCallback("OnFrame")
	runCallback("OnFrame")
end
