base = "attack"
setup = "ring3_graft_setup"
-- Ring 3 and the Graft slots (R30): what PoB's `slot.shown` shows on a 3_27 tree with "+1 Ring
-- Slot" allocated, equipping into each, Ctrl+Shift+click on a list row (the second slot) and
-- Ctrl+click taking an item out, then the node going, which hides Ring 3.
function recipe()
	checkpoint()
	act.equip("Ring 3", pick.item("Third Loop"))
	act.equip("Graft 1", pick.item("Battering Uulgraft"))
	act.equip("Graft 2", pick.item("Spiking Uulgraft"))
	checkpoint()
	-- Ctrl+Shift+click: Ring 1 is the ring's own slot, the second one is Ring 2
	act.quick_equip(pick.item("Fourth Band"), true)
	-- Ctrl+click: Ring 1
	act.quick_equip(pick.item("Fifth Circle"), false)
	checkpoint()
	-- Ctrl+click on what is already there takes it out
	act.quick_equip(pick.item("Fifth Circle"), false)
	act.undo()
	checkpoint()
	-- without the node PoB hides Ring 3
	act.dealloc(build.spec.nodes[43902])
	checkpoint()
end
