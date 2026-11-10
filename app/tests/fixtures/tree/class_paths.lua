base = "attack"
function recipe()
	-- an ascendancy node of another ascendancy of the class
	act.alloc_ascendancy(pick.ascendancy_node())
	-- a class the tree doesn't connect to; Connect Path
	act.class(pick.unconnected_class(), true)
	act.reset()
end
