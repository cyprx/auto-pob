base = "dot"
function recipe()
	local notable = pick.notable(6)
	local middle = pick.path_middle(notable)
	act.alloc(notable)
	act.dealloc(middle)
end
