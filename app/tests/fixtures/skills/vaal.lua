base = "attack"
function recipe()
	act.paste("Vaal Haste 20/0  1\r\n")
	act.toggle(1, 0, 2, false)
	checkpoint()
	act.toggle(1, 0, 2, true)
	act.toggle(1, 0, 1, false)
	checkpoint()
	-- a transfigured gem replaces it; PoB's enabled box resets both flags on a non-Vaal gem
	act.gem(1, 0, "Boneshatter of Complex Trauma")
	act.gem_enabled(1, 0, false)
	act.gem_enabled(1, 0, true)
	act.count(1, 0, "3")
	checkpoint()
end
