base = "dot"
-- The recipe's end state is saved as XML (`save_xml`) for the gate to open in our engine.
save_xml = true
function recipe()
	-- Setup, not recorded: a unique jewel with a Medium radius, socketed the way the socket's
	-- item slot control does (ItemSlotControl.lua:13-18, 94-100).
	local jewel = new("Item", [[Rarity: UNIQUE
Eldritch Knowledge
Cobalt Jewel
Radius: Medium
5% increased Chaos Damage per 10 Intelligence from Allocated Passives in Radius]])
	build.itemsTab:AddItem(jewel, true)
	local socket = pick.socket()
	build.itemsTab.sockets[socket.id]:SetSelItemId(jewel.id)
	build.itemsTab:PopulateSlots()
	build.itemsTab:AddUndoState()
	build.buildFlag = true
	act.alloc(socket)
end
