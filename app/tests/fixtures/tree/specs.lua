base = "attack"
function recipe()
	act.new_spec("Second")
	checkpoint()
	act.select_spec(1)
	act.alloc(pick.notable(2))
	act.select_spec(0)
	act.rename_spec(1, "Renamed")
	act.compare(1)
	checkpoint()
	act.delete_spec(1)
	checkpoint()
end
