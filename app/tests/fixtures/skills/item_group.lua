base = "attack"
function recipe()
	-- Setup, not recorded: a rare helmet that grants Icestorm (a helmet has sockets, so the
	-- item group also shows the sockets line and the Optimise Sockets button), equipped the way
	-- the Items tab would (ItemSlotControl:SetSelItemId), then saved and loaded again so both
	-- sides start from the same build.
	local helmet = new("Item", [[Rarity: RARE
Gale Crown
Iron Hat
Implicits: 0
Grants Level 20 Icestorm Skill]])
	build.itemsTab:AddItem(helmet, true)
	build.itemsTab.slots["Helmet"]:SetSelItemId(helmet.id)
	build.itemsTab:PopulateSlots()
	build.itemsTab:AddUndoState()
	build.buildFlag = true
	runCallback("OnFrame")
	setup_xml = build:SaveDB("item_group")
	loadBuildFromXML(setup_xml, "item_group")
	runCallback("OnFrame")
	local tab = build.skillsTab
	local item
	for i, group in ipairs(tab.socketGroupList) do
		if group.source then item = i - 1 end
	end
	assert(item, "no group came from the item")
	-- the count moves Full DPS
	local function fullDps()
		return build.calcsTab.mainOutput.FullDPS
	end
	act.full_dps(0, true)
	act.full_dps(item, true)
	local one = fullDps()
	act.group_count(item, "3")
	local three = fullDps()
	assert(one ~= three, "the count doesn't change Full DPS: " .. tostring(one))
	act.label(item, "Granted")
	checkpoint()
	act.enable(item, false)
	checkpoint()
end
