base = "attack"
-- Browse PoB's databases: a unique is shown on the display panel and saved, then the first rare
-- template for rings is added with Ctrl+click and the add is undone.
function recipe()
	local tab = build.itemsTab
	act.db_open("Uniques", { search = "Kaom's Heart" })
	-- the database keys an item by its name and base
	local heart = dbDump("Uniques").rows[1]
	assert(heart and heart.name:find("Kaom's Heart", 1, true), "Kaom's Heart is not listed")
	act.db_view("Uniques", heart.name)
	assert(tab.displayItem and tab.displayItem.name:find("Kaom's Heart", 1, true), "Kaom's Heart is not displayed")
	checkpoint()
	act.save()
	assert(tab.displayItem == nil, "the item stayed on the panel")
	checkpoint()

	-- the first row of the rare templates for rings
	act.db_open("Rares", { slot = "Ring" })
	local first = dbDump("Rares").rows[1]
	assert(first, "no ring templates")
	act.db_add("Rares", first.name)
	assert(tab.slots["Ring 1"].selItemId ~= 0, "the ring was not equipped")
	checkpoint()

	act.undo()
	assert(tab.slots["Ring 1"].selItemId == 0, "the undo left the ring equipped")
	checkpoint()
end
