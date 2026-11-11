base = "attack"
-- Item text is literal and recorded verbatim in the actions.
local RING_A = [[Rarity: RARE
Doom Loop
Iron Ring
Implicits: 0
+30 to maximum Life
+12% to Fire Resistance]]
local RING_B = [[Rarity: RARE
Gale Band
Paua Ring
Implicits: 0
+25 to maximum Mana
+10% to Cold Resistance]]
local ARMOUR_1 = [[Rarity: UNIQUE
Kaom's Heart
Glorious Plate
Implicits: 0
+500 to maximum Life]]
local ARMOUR_2 = [[Rarity: UNIQUE
Skin of the Lords
Simple Robe
Implicits: 0
+90 to maximum Life
+20% to all Elemental Resistances]]

function recipe()
	-- a new item: "Add to build" and no id; Save puts it in the first empty ring slot
	act.paste(RING_A)
	checkpoint()
	act.save()
	local ringA = pick.item("Doom Loop")
	-- in Ring 2 instead
	act.equip("Ring 1", nil)
	act.equip("Ring 2", ringA)

	-- the second body armour goes to the list only; then it replaces the first
	act.paste(ARMOUR_1)
	act.save()
	act.paste(ARMOUR_2)
	act.save()
	act.equip("Body Armour", pick.item("Skin of the Lords"))
	checkpoint()

	-- a second ring, auto-equipped in Ring 1
	act.paste(RING_B)
	act.save()
	local ringB = pick.item("Gale Band")

	-- editing the first ring: one more mod line, saved in place
	act.edit(ringA)
	act.edit_text(2, displayDump().edit_text .. "\n+15 to maximum Mana")
	checkpoint()
	act.save()
	checkpoint()

	-- the display tooltip follows the outputs while an item is displayed
	act.edit(ringA)
	act.equip("Ring 2", ringB)
	checkpoint()
	-- Ctrl+D with an item displayed
	act.stat_differences(false)
	checkpoint()
	act.cancel()
	act.equip("Ring 2", nil)
	checkpoint()
end
