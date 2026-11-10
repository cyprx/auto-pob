base = "attack"
function recipe()
	act.paste("Vaal Haste 20/0  1\r\n")
	act.toggle(1, 0, 2, false)
	act.toggle(1, 0, 1, false)
	checkpoint()
	-- a transfigured gem replaces it; its count is shown
	act.gem(1, 0, "Boneshatter of Complex Trauma")
	act.count(1, 0, "3")
	checkpoint()
end
