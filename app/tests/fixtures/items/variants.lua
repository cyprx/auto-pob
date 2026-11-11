base = "attack"
-- The three shapes of variant control (lesson 8). Item text is recorded verbatim in the actions.

-- A hand-written item with two versions: no PoB unique uses `Version:` lines
local VERSIONED = [[Rarity: UNIQUE
Veil of Versions
Prismatic Ring
Version: Old
Version: New
Implicits: 0
{version:1}+20 to maximum Life
{version:2}+60 to maximum Life
{version:1,2}+10% to Fire Resistance
{version:2}+15 to maximum Mana]]

local function itemText(name)
	for _, list in pairs(data.uniques) do
		for _, text in ipairs(list) do
			if text:match("^[^\n]+") == name then
				return text
			end
		end
	end
	error("no unique " .. name)
end

-- The first legacy unique with an alternate variant, by name. Jewels are left out: they go to
-- a tree socket, which this recipe does not allocate.
local function firstAltVariant()
	local names = {}
	for _, list in pairs(data.uniques) do
		for _, text in ipairs(list) do
			local base = data.itemBases[text:match("^[^\n]+\n([^\n]+)")]
			if base and base.type ~= "Jewel" and text:find("\nHas Alt Variant: true", 1, true)
				and select(2, text:gsub("\nVariant:", "")) >= 3 then
				table.insert(names, text:match("^[^\n]+"))
			end
		end
	end
	table.sort(names)
	return names[1]
end

function recipe()
	local tab = build.itemsTab

	-- a legacy unique: variant 2, alternate variant 3
	local name = firstAltVariant()
	act.paste(itemText(name))
	assert(tab.displayItem.hasAltVariant, name .. " has no alternate variant")
	assert(not tab.displayItem.usesVariantGroups, name .. " uses variant groups")
	act.variant(0, 1)
	act.variant(1, 2)
	checkpoint()
	act.save()
	local legacy = pick.item(name)
	act.equip(tab.items[legacy]:GetPrimarySlot(), legacy)
	checkpoint()

	-- variant groups: the second group's control to its second choice
	act.paste(itemText("Zana's Ingenuity"))
	assert(tab.displayItem.usesVariantGroups, "Zana's Ingenuity has no variant groups")
	act.variant(1, 1)
	checkpoint()

	-- versions: version 2 is the default, so pick the first and then the second again
	act.paste(VERSIONED)
	assert(tab.displayItem.usesVariantGroups and #tab.displayItem.versionList == 2, "no versions")
	act.version(0)
	checkpoint()
	act.version(1)
	act.save()
	checkpoint()
end
