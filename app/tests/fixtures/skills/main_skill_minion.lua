base = "minion"
function recipe()
	act.label(0, "Main")
	checkpoint()
	act.sidebar("mainSkillMinion", 0)
	checkpoint()
	act.sidebar("mainSkillMinionSkill", 1)
	checkpoint()
end
