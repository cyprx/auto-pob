base = "attack"
function recipe()
	-- edit group 1 so that groups that aren't the first are covered
	act.paste("Cleave 20/0  1\r\n")
	act.gem(1, 1, "Melee Splash")
	act.level(1, 1, "18")
	act.quality(1, 1, "20")
	act.count(1, 0, "2")
	act.gem_enabled(1, 0, false)
	checkpoint()
	-- empty text falls back: the natural max level, the default quality, a count of 1
	act.level(1, 1, "")
	act.quality(1, 1, "")
	act.count(1, 0, "")
	act.gem_enabled(1, 0, true)
	checkpoint()
	act.gem(1, 2, "Fortify")
	checkpoint()
	act.delete_gem(1, 1)
	-- a name no gem has: a paste keeps it, with an error
	act.paste("Fireblal 20/0  1\r\n")
	act.level(1, 0, "")
	checkpoint()
	act.undo()
	checkpoint()
end
