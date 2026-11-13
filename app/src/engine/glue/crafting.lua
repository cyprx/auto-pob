-- The display panel's crafting sections (R3, R9, R11, R12): the windows first, then the affix
-- editor, the cluster jewel controls and the custom-mod list.

glue = glue or {}

local NOT_SHOWN = "PoB doesn't show that control for this item"
local MAX_AFFIXES = 9

local function items_tab()
	return pob.build.itemsTab
end

-- A dropdown as the panel shows it; `enabled` is the control's own, which can be a function
local function drop_view(control)
	local view = glue.drop_list(control)
	view.enabled = control:IsEnabled() and true or false
	return view
end

-- the label of a dropdown entry: "None" is a string, the rest are tables
local function entry_label(entry)
	return type(entry) == "table" and entry.label or entry
end

-- the windows PoB's panel offers for this item
table.insert(glue.display_sections, function(tab, item, out)
	out.popups = glue.panel_popups(tab)
end)

-- The shown affix dropdowns with their sliders (R9). The label drops its colour code.
table.insert(glue.display_sections, function(tab, item, out)
	out.affixes = {}
	for i = 1, MAX_AFFIXES do
		local drop = tab.controls["displayItemAffix" .. i]
		if drop:IsShown() then
			local label = tab.controls["displayItemAffixLabel" .. i]:GetProperty("label")
			local affix = {
				slot = i - 1,
				label = (label:gsub("^%^7", "")),
				list = drop_view(drop),
			}
			if drop.slider:IsShown() then
				affix.range = drop.slider.val
				affix.tiers = drop.slider.divCount
			end
			table.insert(out.affixes, affix)
		end
	end
end)

-- "Modifier sorting" and the cluster jewel controls, while PoB shows them
table.insert(glue.display_sections, function(tab, item, out)
	local c = tab.controls
	if c.craftingSorting:IsShown() then
		out.crafting_sort = drop_view(c.craftingSorting)
	end
	if c.displayItemClusterJewelSkill:IsShown() then
		out.cluster = {
			skill = drop_view(c.displayItemClusterJewelSkill),
			count = item.clusterJewelNodeCount,
			min = item.clusterJewel.minNodes,
			max = item.clusterJewel.maxNodes,
		}
	end
end)

-- The lines PoB's Remove rows list (`UpdateCustomControls`), read from the model. The rows'
-- own labels are cut by `DrawStringCursorIndex`, which is stubbed headless (R12).
local function custom_lines(item)
	local lines = copyTable(item.explicitModLines)
	local crucible = item.crucibleModLines
	if crucible and #crucible > 0 then
		for _, line in ipairs(crucible) do
			table.insert(lines, line)
		end
	end
	local out = {}
	if item.rareLikeUnique or item.rarity == "MAGIC" or item.rarity == "RARE"
		or (crucible and #crucible > 0) then
		for _, modLine in ipairs(lines) do
			if modLine.custom or modLine.crafted or modLine.crucible then
				local line = itemLib.formatModLine(modLine)
				if line then
					table.insert(out, {
						kind = modLine.crafted and "Crafted" or modLine.crucible and "Crucible" or "Custom",
						line = line,
					})
				end
			end
		end
	end
	return out
end

table.insert(glue.display_sections, function(tab, item, out)
	out.custom_mods = custom_lines(item)
end)

-- ---- Commands (R9, R11, R12) ----

local function display_tab()
	local tab = items_tab()
	if not tab.displayItem then
		error("no item is being viewed", 0)
	end
	return tab
end

-- A shown, enabled control, or the error PoB's panel would give by not offering it
local function usable(control)
	if not (control and control:IsShown() and control:IsEnabled()) then
		error(NOT_SHOWN, 0)
	end
	return control
end

-- The affix dropdown of `slot` (0..8)
local function affix_drop(tab, slot)
	return usable(tab.controls["displayItemAffix" .. (slot + 1)])
end

-- Picks entry `index` (0-based) of an affix dropdown. PoB rebuilds every list after a pick,
-- so the entry must still be the one the caller saw (R8).
function glue.affix_set(slot, index, label)
	local drop = affix_drop(display_tab(), slot)
	local entry = drop.list[index + 1]
	if entry == nil then
		error("no such choice", 0)
	end
	if entry_label(entry) ~= label then
		error("the list changed; pick again", 0)
	end
	drop:SetSel(index + 1)
end

-- The slider under an affix dropdown
function glue.affix_range(slot, value)
	local drop = affix_drop(display_tab(), slot)
	usable(drop.slider):SetVal(value)
end

-- "Modifier sorting": PoB sorts in the selFunc, synchronously (R10)
function glue.crafting_sort(index)
	local control = usable(display_tab().controls.craftingSorting)
	if index < 1 or index > #control.list then
		error("no such choice", 0)
	end
	control:SetSel(index)
end

function glue.cluster_skill(index)
	local control = usable(display_tab().controls.displayItemClusterJewelSkill)
	if index < 1 or index > #control.list then
		error("no such choice", 0)
	end
	control:SetSel(index)
end

-- The node count slider takes 0..1 across the jewel's range
function glue.cluster_count(count)
	local tab = display_tab()
	usable(tab.controls.displayItemClusterJewelSkill)
	local slider = usable(tab.controls.displayItemClusterJewelNodeCount)
	local jewel = tab.displayItem.clusterJewel
	if count < jewel.minNodes or count > jewel.maxNodes then
		error("no such choice", 0)
	end
	slider:SetVal((count - jewel.minNodes) / (jewel.maxNodes - jewel.minNodes))
end

-- Removes custom mod `index` (1-based, as the view lists them): the Remove row's own button,
-- after the sync PoB's Draw would have made (R12)
function glue.remove_custom(index)
	local tab = display_tab()
	tab:UpdateCustomControls()
	local button = tab.controls["displayItemCustomModifierRemove" .. index]
	if index < 1 or not (button and button:IsShown()) then
		error(NOT_SHOWN, 0)
	end
	button.onClick()
end

-- The tooltip of an affix entry (`index`, 0-based) or, without one, of the slider, as PoB
-- builds it on hover: for the value under the pointer (`at`, 0..1), or the slider's own.
-- Nil when the control or the entry is gone, or while a popup is open.
function glue.affix_tooltip(slot, index, at)
	local tab = items_tab()
	-- PoB blanks these tooltips while a popup is open, and a session is one (R1)
	if not tab.displayItem or glue.popup_live() then
		return nil
	end
	local drop = tab.controls["displayItemAffix" .. (slot + 1)]
	if not (drop and drop:IsShown()) then
		return nil
	end
	local tooltip = new("Tooltip")
	if index then
		local entry = drop.list[index + 1]
		if entry == nil then
			return nil
		end
		drop.tooltipFunc(tooltip, "HOVER", index + 1, entry)
	else
		if not drop.slider:IsShown() then
			return nil
		end
		drop.slider.tooltipFunc(tooltip, at or drop.slider.val)
	end
	return glue.read_tooltip(tooltip)
end
