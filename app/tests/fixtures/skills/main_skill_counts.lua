base = "attack"
function recipe()
	-- Scorching Ray shows a Stages box; typing drops what isn't a digit
	act.paste("Scorching Ray 20/0  1\r\n")
	act.main_group(1)
	checkpoint()
	act.sidebar_count("mainSkillStageCount", "5x6")
	checkpoint()
	act.sidebar_count("mainSkillStageCount", "3")
	checkpoint()
end
