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
	add([[Rarity: RARE
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

	-- a life flask, switched on with its own checkbox
	add([[Rarity: NORMAL
Divine Life Flask]], "Flask 1")
	local activate = tab.slots["Flask 1"].controls.activate
	activate.state = true
	activate.changeFunc(true)

	-- an unused magic jewel, and a unique one in an allocated tree socket
	add([[Rarity: MAGIC
Cobalt Jewel
Implicits: 0
+10 to Intelligence]])
	local jewel = add([[Rarity: UNIQUE
Eldritch Knowledge
Cobalt Jewel
Radius: Medium
5% increased Chaos Damage per 10 Intelligence from Allocated Passives in Radius]])
	local socket = pick.socket()
	act.alloc(socket)
	tab.sockets[socket.id]:SetSelItemId(jewel.id)
	tab:PopulateSlots()

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
