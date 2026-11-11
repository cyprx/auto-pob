base = "attack"
-- Item text is literal and recorded verbatim in the actions.
local AMULET = [[Rarity: RARE
Doom Pendant
Onyx Amulet
Quality: 0
Implicits: 1
{tags:attribute}+12 to all Attributes
{tags:life}+70 to maximum Life
{tags:resistance}+30% to Fire Resistance]]
local ARMOUR = [[Rarity: RARE
Doom Plate
Glorious Plate
Sockets: R-R-R-R
Implicits: 0
+50 to maximum Life]]

-- The first unique, by name, that Foulborn can change and that has a slider as well
local function foulbornUnique()
	local titles = {}
	for title in pairs(data.foulbornMap) do
		table.insert(titles, title)
	end
	table.sort(titles)
	local texts = {}
	for _, list in pairs(data.uniques) do
		for _, text in ipairs(list) do
			texts[text:match("^[^\n]+")] = text
		end
	end
	for _, title in ipairs(titles) do
		local text = texts[title]
		if text then
			local item = new("Item", text)
			local mutable, sliders = nil, 0
			for i, modLine in ipairs(item.rangeLineList or {}) do
				if modLine.modId and modLine.newModId and not mutable then mutable = i end
				if modLine.showSlider then sliders = sliders + 1 end
			end
			local lines = #item.rangeLineList
			if mutable and sliders >= 2 and #item.explicitModLines > 0 and lines >= 2 then
				return text, mutable - 1, title
			end
		end
	end
	error("no suitable foulborn unique")
end

function recipe()
	local tab = build.itemsTab

	-- an amulet with tagged mods: quality, both influences, a catalyst and its quality
	act.paste(AMULET)
	local c = tab.controls
	assert(c.displayItemCatalyst:IsShown(), "the catalyst dropdown is not shown")
	assert(c.displayItemInfluence:IsShown(), "the influence dropdown is not shown")
	act.quality("20")
	act.quality("abc")
	act.influence(0, 1)
	act.influence(1, 2)
	act.catalyst(4)
	act.catalyst_quality("15")
	checkpoint()
	act.save()
	checkpoint()

	-- a body armour with four sockets: the first goes green, the second and third unlink, one is added
	act.paste(ARMOUR)
	assert(#tab.displayItem.sockets == 4, "not four sockets")
	act.socket(0, "G")
	act.link(1, false)
	act.add_socket()
	checkpoint()
	act.save()

	-- a unique with range lines and a Foulborn line
	local text, mutable, title = foulbornUnique()
	act.paste(text)
	local item = tab.displayItem
	act.range(1, 0.25)
	checkpoint()
	act.range(0, 1.0)
	-- the same unique again: its first slider still holds 1.0 from the last item, and PoB syncs
	-- it to the new line only when the slider asks if it is shown
	act.paste(text)
	item = tab.displayItem
	local stale = c.displayItemStackedRangeSlider1
	assert(stale.val ~= item.rangeLineList[1].range, "the slider is not stale")
	assert(stale.val == 1.0, "the slider doesn't hold the new value")
	act.range(0, 1.0)
	assert(item.rangeLineList[1].range == 1.0, "the range didn't change")
	checkpoint()
	act.mutated(mutable, true)
	checkpoint()
	act.toggle_mod("explicit", 0)
	assert(tab.displayItem.explicitModLines[1].disabled, "the first explicit line is not off")
	checkpoint()
	act.save()
	-- Foulborn renames the item, so look for the title inside the name
	local id
	for itemId, saved in pairs(tab.items) do
		if saved.name:find(title, 1, true) then id = itemId end
	end
	assert(id, "the unique was not saved")
	act.equip(tab.items[id]:GetPrimarySlot(), id)
	checkpoint()
end
