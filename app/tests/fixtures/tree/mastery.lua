base = "attack"
function recipe()
	local node = pick.mastery()
	act.mastery(node, pick.mastery_effects(node)[2])
	act.undo()
	act.redo()
end
