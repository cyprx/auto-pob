base = "attack"
-- A click on a bloodline node chooses that bloodline and allocates the node
-- (PassiveTreeView.lua:398-411, 499-505), even over another chosen bloodline
function recipe()
	act.alloc_bloodline(pick.bloodline_node("Trialmaster"))
	checkpoint({ sidebar = true })
	act.alloc_bloodline(pick.bloodline_next("Trialmaster"))
	act.alloc_bloodline(pick.bloodline_node("Farrul"))
	checkpoint({ sidebar = true })
	act.undo()
	checkpoint()
end
