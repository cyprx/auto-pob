base = "attack"
-- A setup-only script for the Items gate tests (no recorded actions): it fills the build with
-- the kinds of items and slots the Items tab shows, then saves it as `setup_xml` for both sides
-- to open. Everything is done at model level, as tree/jewel.lua does; item text is literal.
function recipe()
	local tab = build.itemsTab
	local function add(text, slot)
		local item = new("Item", text)
		tab:AddItem(item, true)
		if slot then
			tab.slots[slot]:SetSelItemId(item.id)
			tab:PopulateSlots()
		end
		return item
	end

	-- a rare ring in Ring 1 and one that nothing uses
	add([[Rarity: RARE
Doom Loop
Iron Ring
Implicits: 0
+30 to maximum Life]], "Ring 1")
	-- (equipped in Ring 2 of the second item set only, below)
	local paua = add([[Rarity: RARE
Gale Band
Paua Ring
Implicits: 0
+25 to maximum Mana]])

	-- a unique body armour
	add([[Rarity: UNIQUE
Kaom's Heart
Glorious Plate
Implicits: 0
+500 to maximum Life]], "Body Armour")

	-- a belt with an abyssal socket (populating makes the socket active), holding an abyss jewel
	local belt = add([[Rarity: RARE
Doom Cord
Stygian Vise
Implicits: 1
Has 1 Abyssal Socket
+40 to maximum Life]], "Belt")
	assert(belt.abyssalSocketCount == 1, "the belt has no abyssal socket")
	add([[Rarity: RARE
Gloom Eye
Murderous Eye Jewel
Implicits: 0
+12 to maximum Life]], "Belt Abyssal Socket 1")
	-- an abyss jewel that only the second item set holds (below)
	local eye = add([[Rarity: RARE
Dusk Eye
Murderous Eye Jewel
Implicits: 0
+14 to maximum Mana]])

	-- a life flask, switched on with its own checkbox
	add([[Rarity: NORMAL
Divine Life Flask]], "Flask 1")
	local activate = tab.slots["Flask 1"].controls.activate
	activate.state = true
	activate.changeFunc(true)

	-- a magic jewel in an allocated tree socket, and an unused one
	local magic = add([[Rarity: MAGIC
Cobalt Jewel
Implicits: 0
+10 to Intelligence]])
	add([[Rarity: MAGIC
Viridian Jewel
Implicits: 0
+10 to Dexterity]])
	local socket = pick.socket()
	act.alloc(socket)
	tab.sockets[socket.id]:SetSelItemId(magic.id)
	tab:PopulateSlots()

	-- a second tree, PoB's own copy of this one (Convert makes it and shows it), where the
	-- unique jewel sits in the same socket; the first tree is shown again afterwards
	local jewel = add([[Rarity: UNIQUE
Eldritch Knowledge
Cobalt Jewel
Radius: Medium
5% increased Chaos Damage per 10 Intelligence from Allocated Passives in Radius]])
	local treeTab = build.treeTab
	treeTab:ConvertToVersion(build.spec.treeVersion)
	treeTab.specList[2].title = "Other tree"
	tab.sockets[socket.id]:SetSelItemId(jewel.id)
	tab:PopulateSlots()
	treeTab:SetActiveSpec(1)
	tab:PopulateSlots()
	assert(build.spec.jewels[socket.id] == magic.id and treeTab.specList[2].jewels[socket.id] == jewel.id)

	-- a second item set, a copy of set 1 as the set manager's Copy makes it, that uses the
	-- second weapon set and has a one-hand weapon in "Weapon 1 Swap"
	local sword = add([[Rarity: RARE
Doom Edge
Rusted Sword
Implicits: 0
Adds 1 to 4 Physical Damage]])
	local copy = copyTable(tab.itemSets[1])
	copy.id = 2
	copy.title = "Swap"
	copy.useSecondWeaponSet = true
	-- what set 1 lacks, in slots that aren't swap slots
	copy["Ring 2"].selItemId = paua.id
	copy["Belt Abyssal Socket 1"].selItemId = eye.id
	copy["Weapon 1 Swap"].selItemId = sword.id
	tab.itemSets[2] = copy
	table.insert(tab.itemSetOrderList, 2)

	tab:AddUndoState()
	build.buildFlag = true
	runCallback("OnFrame")
	assert(tab.activeItemSetId == 1, "set 1 should be active")
	setup_xml = build:SaveDB("gear")
	loadBuildFromXML(setup_xml, "gear")
	runCallback("OnFrame")
end
