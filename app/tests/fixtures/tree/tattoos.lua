base = "attack"
-- Tattoos through the right-click window (TreeTab:ModifyNodePopup) and "Remove All Tattoos".
-- A small Strength node and a keystone are allocated first, so their tattoos count in the
-- sidebar.

local function tattooed()
	return { tattoos = true, sidebar = true }
end

-- Chooses entry `index` (1-based) of the open window's Modifier dropdown
local function chooseModifier(index)
	local list = main.popups[1].controls.modSelect.list
	act.popup_set("modSelect", { Choice = { index = index - 1, label = list[index].label } })
end

function recipe()
	local strength = pick.attribute("Strength")
	local keystone = pick.keystone()
	act.alloc(strength)
	act.alloc(keystone)

	-- 1. the Strength node takes the second tattoo
	act.tattoo_open(strength)
	chooseModifier(2)
	act.popup_press("save")
	checkpoint(tattooed())

	-- 2. the keystone takes the first, with the legacy tattoos listed
	act.tattoo_open(keystone)
	act.popup_set("showLegacyTattoo", { Check = true })
	chooseModifier(1)
	act.popup_press("save")
	checkpoint(tattooed())

	-- 3. Reset Node on the Strength node
	act.tattoo_open(strength)
	act.popup_press("reset")
	checkpoint(tattooed())

	-- 4. Undo restores both tattoos
	act.undo()
	checkpoint(tattooed())

	-- 5. Remove All Tattoos adds no undo state
	act.remove_all_tattoos()
	checkpoint(tattooed())
end
