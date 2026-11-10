base = "attack"
function recipe()
	-- the imbued support needs a socketed item: the Driftwood Maul
	act.slot(0, "Weapon 1")
	act.imbued(0, "Added Fire Damage")
	checkpoint()
	act.optimise(0)
	checkpoint()
	act.imbued_clear(0)
	checkpoint()
end
