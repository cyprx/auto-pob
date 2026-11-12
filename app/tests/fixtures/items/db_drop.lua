base = "attack"
-- The databases' other two ways in (R23): Ctrl+Shift+click on the first rare ring template,
-- which PoB equips into Ring 2, then the first unique ring dragged onto Ring 1 and released,
-- and the drop undone.
function recipe()
	local tab = build.itemsTab
	act.db_open("Rares", { slot = "Ring" })
	local rare = dbDump("Rares").rows[1]
	assert(rare, "no ring templates")
	act.db_add("Rares", rare.name, true)
	assert(tab.slots["Ring 2"].selItemId ~= 0, "the ring is not in Ring 2")
	checkpoint()

	act.db_open("Uniques", { slot = "Ring" })
	local unique = dbDump("Uniques").rows[1]
	assert(unique, "no unique rings")
	local before = tab.slots["Ring 1"].selItemId
	act.db_drop("Uniques", unique.name, "Ring 1")
	assert(tab.slots["Ring 1"].selItemId ~= before, "the drop equipped nothing")
	checkpoint()

	act.undo()
	assert(tab.slots["Ring 1"].selItemId == before, "the undo left the ring")
	checkpoint()
end
