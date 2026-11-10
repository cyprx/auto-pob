base = "attack"
function recipe()
	-- a second group, then the group list's right-click makes it main
	act.paste("Cleave 20/0  1\r\n")
	checkpoint()
	act.main_group(1)
	checkpoint()
	-- the sidebar goes back to the first group and picks Boneshatter's second part
	act.sidebar("mainSocketGroup", 0)
	act.sidebar("mainSkillPart", 1)
	checkpoint()
	-- the right-click is undoable, the sidebar choices only set the unsaved flag
	act.undo()
	checkpoint()
end
