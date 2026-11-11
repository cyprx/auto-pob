base = "attack"
setup = "gear"
function recipe()
	local t = build.itemsTab
	local first = t.itemSetOrderList[1]
	local mapping = act.new_set("Mapping")
	local bossing = act.copy_set(first, "Bossing")
	checkpoint()
	act.select_set(bossing)
	-- a ring that is in the item list but not in this set's Ring 2
	act.equip("Ring 2", pick.item("Gale Band"))
	act.weapon_set(true)
	checkpoint()
	act.rename_set(mapping, "Clear")
	act.move_set(2, 0)
	checkpoint()
	-- the active set goes: PoB picks the previous one
	act.delete_set(t.activeItemSetId)
	act.undo()
	checkpoint()
end
