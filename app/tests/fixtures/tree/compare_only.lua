base = "attack"
-- A large cluster jewel's subgraph allocated in the first tree only: the second tree, compared
-- with the first, shows those nodes and their tooltips (PassiveTreeView.lua:1102-1153, 1748)
function recipe()
	-- Setup, not recorded: the jewel in the nearest outer socket and its notables allocated
	local spec = build.spec
	local outer
	for _, node in pairs(spec.nodes) do
		if node.type == "Socket" and node.expansionJewel and node.expansionJewel.size == 2
			and node.path and (not outer or node.pathDist < outer.pathDist
				or (node.pathDist == outer.pathDist and node.id < outer.id)) then
			outer = node
		end
	end
	assert(outer, "no outer socket")
	spec:AllocNode(outer, outer.path)
	local jewel = new("Item", [[Rarity: RARE
Gate Cluster
Large Cluster Jewel
Item Level: 84
LevelReq: 54
Implicits: 3
Adds 8 Passive Skills
2 Added Passive Skills are Jewel Sockets
Added Small Passive Skills grant: 12% increased Fire Damage
1 Added Passive Skill is Burning Bright
1 Added Passive Skill is Prismatic Heart
+10 to maximum Life]])
	build.itemsTab:AddItem(jewel, true)
	build.itemsTab.slots["Jewel " .. outer.id]:SetSelItemId(jewel.id)
	spec:BuildClusterJewelGraphs()
	runCallback("OnFrame")
	local added = 0
	for id, node in pairs(spec.nodes) do
		if id >= 65536 and node.path and not node.alloc and node.type == "Notable" then
			spec:AllocNode(node, node.path)
			added = added + 1
		end
	end
	assert(added > 0, "no cluster notable reached")
	spec:AddUndoState()
	build.buildFlag = true
	runCallback("OnFrame")
	setup_xml = build:SaveDB("compare_only")
	loadBuildFromXML(setup_xml, "compare_only")

	act.new_spec("Plain")
	act.select_spec(1)
	act.compare(0)
	checkpoint()
end
