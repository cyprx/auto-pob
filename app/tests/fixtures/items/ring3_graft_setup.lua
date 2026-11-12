base = "attack"
-- A setup-only script for `ring3_graft` (no recorded actions): the attack build moved to the
-- 3_27 tree, where the Graft slots show (ItemsTab.lua:166-169) and the King in the Mists
-- bloodline's "The Unseen Hand" (node 43902) grants "+1 Ring Slot", the only source of
-- `AdditionalRingSlot` in PoB 2.67.2 (TreeData/3_27/tree.lua:12793-12805). Then three spare
-- rings and two grafts, none equipped. Model level, as gear.lua; item text is literal.
function recipe()
	local xml = build:SaveDB("ring3_graft"):gsub('treeVersion="3_29"', 'treeVersion="3_27"')
	loadBuildFromXML(xml, "ring3_graft")
	local spec = build.spec
	assert(spec.treeVersion == "3_27", "the tree is " .. tostring(spec.treeVersion))
	local king
	for id, ascend in pairs(spec.tree.alternate_ascendancies) do
		if ascend.id == "KingInTheMists" then king = id end
	end
	assert(king, "no King in the Mists bloodline on 3_27")
	spec:SelectSecondaryAscendClass(king)
	spec:BuildAllDependsAndPaths()
	local hand = spec.nodes[43902]
	assert(hand and hand.path, "The Unseen Hand can't be reached")
	spec:AllocNode(hand)
	spec:AddUndoState()

	local tab = build.itemsTab
	for _, text in ipairs({
		"Rarity: RARE\nThird Loop\nIron Ring\nImplicits: 0\n+31 to maximum Life",
		"Rarity: RARE\nFourth Band\nCoral Ring\nImplicits: 0\n+32 to maximum Life",
		"Rarity: RARE\nFifth Circle\nPaua Ring\nImplicits: 0\n+33 to maximum Mana",
		"Rarity: NORMAL\nBattering Uulgraft",
		"Rarity: NORMAL\nSpiking Uulgraft",
	}) do
		tab:AddItem(new("Item", text), true)
	end
	tab:PopulateSlots()
	tab:AddUndoState()
	build.buildFlag = true
	runCallback("OnFrame")
	assert(tab.slots["Ring 3"]:IsShown(), "Ring 3 is not shown")
	assert(tab.slots["Graft 1"]:IsShown(), "Graft 1 is not shown")
	setup_xml = build:SaveDB("ring3_graft")
	loadBuildFromXML(setup_xml, "ring3_graft")
	runCallback("OnFrame")
end
