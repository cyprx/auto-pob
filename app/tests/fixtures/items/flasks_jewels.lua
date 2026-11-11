base = "attack"
setup = "gear"
function recipe()
	-- PoB's undo restores the sets' copies, so the flask's live state is what the checkpoint
	-- shows after the undo (R12)
	act.flask("Flask 1", false)
	act.undo()
	checkpoint()
	-- that undo also took the magic jewel out of its socket, since the build's first undo
	-- state predates it (it is PoB's, and the checkpoint shows it): put it back
	local held
	for node, id in pairs(build.treeTab.specList[2].jewels) do
		if id ~= 0 then held = node end
	end
	act.equip("Jewel " .. held, pick.item("Cobalt Jewel"))
	act.flask("Flask 1", false)
	act.flask("Flask 1", true)
	-- the unused Viridian jewel into a second socket, allocated first
	local socket = pick.socket()
	act.alloc(socket)
	act.equip("Jewel " .. socket.id, pick.item("Viridian Jewel"))
	checkpoint()
	-- the socket's node goes: the slot hides and the jewel stays in `spec.jewels`
	act.dealloc(socket)
	checkpoint()
	act.equip("Belt Abyssal Socket 1", nil)
	act.equip("Belt Abyssal Socket 1", pick.item("Gloom Eye"))
	-- a group in Weapon 1 (the main one) and one in its swap slot: the switch moves the main
	-- group to the swap set's (ItemsTab.lua:224-263)
	act.add_group()
	act.group_slot(0, "Weapon 1")
	act.group_slot(1, "Weapon 1 Swap")
	act.weapon_set(true)
	checkpoint()
	act.undo()
	act.undo()
	checkpoint()
end
