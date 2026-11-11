base = "attack"
setup = "gear"
function recipe()
	act.sort()
	checkpoint()
	-- "Unused Items": only what no slot, set or tree uses; the list can't be reordered
	act.filter(2)
	checkpoint()
	act.filter(0)
	act.move(3, 0)
	-- the equipped body armour: PoB asks first
	act.delete(pick.item("Kaom's Heart"))
	checkpoint()
	act.delete_unused()
	act.undo()
	checkpoint()
	act.redo()
	act.delete_all()
	act.undo()
	checkpoint()
end
