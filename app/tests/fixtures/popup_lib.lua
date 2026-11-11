-- Helpers for the popup tests. Runs in unmodified upstream PoB after `items_lib.lua`.
-- Written independently of glue/popups.lua: the popup stays in `main.popups` as PoB keeps it,
-- every act presses PoB's own control on `main.popups[1]` and runs a frame (which draws the
-- popup), and `popupDump()` reads the controls it finds there. Nothing here may call our glue.

local function frame()
	runCallback("OnFrame")
end

local function record(entry)
	table.insert(actions, entry)
	frame()
end

-- the window each opener button opens, as the gate's `PopupKind`
local openerKinds = {
	craftDisplayItem = "Craft",
	displayItemEnchant = { Enchant = { second = false } },
	displayItemEnchant2 = { Enchant = { second = true } },
	displayItemAnoint = { Anoint = { slot = 0 } },
	displayItemAnoint2 = { Anoint = { slot = 1 } },
	displayItemAnoint3 = { Anoint = { slot = 2 } },
	displayItemAnoint4 = { Anoint = { slot = 3 } },
	displayItemCorrupt = "Corrupt",
	displayItemAddImplicit = "AddImplicit",
	displayItemAddCustom = "AddModifier",
	displayItemAddCrucible = "AddCrucible",
}

-- the kind of the window the last act opened; a window a press opens in its place is the item
-- text editor (Craft's Create)
local popupKind

-- Presses a button of the Items tab that opens a window
function act.open_popup(button)
	local control = build.itemsTab.controls[button]
	assert(control and control:IsShown(), button .. " is not shown")
	local depth = #main.popups
	control.onClick()
	assert(#main.popups == depth + 1, button .. " opened no window")
	popupKind = openerKinds[button]
	record({ cmd = "OpenPopup", kind = popupKind })
end

-- a control of the open popup by name; a list's children are `parent.child`
local function popupControl(name)
	local popup = main.popups[1]
	assert(popup, "no popup is open")
	local parent, child = name:match("^([^.]+)%.(.+)$")
	if parent then
		return popup.controls[parent].controls[child]
	end
	return popup.controls[tonumber(name) or name]
end

-- Sets a control as the user would; `value` is a gate `PopupInput`:
-- { Choice = { index, label } }, { Number = n }, { Check = b }, { Text = s } or { Row = key }
function act.popup_set(name, value)
	local control = popupControl(name)
	assert(control and control:IsShown() and control:IsEnabled(), name .. " is not usable")
	if value.Choice then
		local entry = control.list[value.Choice.index + 1]
		assert((type(entry) == "table" and entry.label or entry) == value.Choice.label, "the list changed")
		control:SetSel(value.Choice.index + 1)
	elseif value.Number then
		control:SetVal(value.Number)
	elseif value.Check ~= nil then
		control.state = value.Check
		if control.changeFunc then control.changeFunc(value.Check) end
	elseif value.Text then
		control:SetText(value.Text, true)
	elseif value.Row then
		for i, entry in ipairs(control.list) do
			local key = type(entry) == "table" and tostring(entry.id or i) or tostring(entry)
			if key == value.Row then
				control:SelectIndex(i)
				control:OnSelClick(i, entry, false)
			end
		end
	else
		error("no such input")
	end
	record({ cmd = "SetPopupControl", name = name, value = value })
end

-- Presses a button of the open popup
function act.popup_press(name)
	local control = popupControl(name)
	assert(control and control:IsShown() and control:IsEnabled(), name .. " is not usable")
	local before = main.popups[1]
	control.onClick()
	if main.popups[1] and main.popups[1] ~= before then popupKind = "EditText" end
	record({ cmd = "PressPopupButton", name = name })
end

-- Cancel and Escape (PopupDialog.lua:72-78)
function act.popup_close()
	main:ClosePopup()
	popupKind = nil
	record({ cmd = "ClosePopup" })
end

-- ---- Dumps ----

local controlKinds = {
	LabelControl = "Label", ButtonControl = "Button", DropDownControl = "Dropdown",
	SliderControl = "Slider", CheckBoxControl = "Check", EditControl = "Edit",
	ListControl = "List", NotableDBControl = "List", ItemDBControl = "List",
}

local function text(value)
	return type(value) == "string" and value or ""
end

local function rowKey(value, index)
	if type(value) == "table" then return tostring(value.id or index) end
	return tostring(value)
end

-- the open popup as plain tables in the shape of `PopupView`, or nil when none is open
function popupDump()
	local popup = main.popups[1]
	if not popup then return nil end
	local px, py = popup:GetPos()
	local controls = {}
	local function add(name, control)
		local kind = controlKinds[control._className]
		if not kind then return end
		local x, y = control:GetPos()
		local w, h = control:GetSize()
		local entry = {
			name = name, kind = kind, label = text(control:GetProperty("label")),
			pos = { x - px, y - py }, size = { w or 0, h or 0 },
			shown = control:IsShown() and true or false, enabled = control:IsEnabled() and true or false,
			choices = {}, selected = 0, value = 0, checked = false, text = "",
			hover_tooltip = kind ~= "List" and control.tooltipFunc ~= nil, rows = {},
		}
		local tip = control:GetProperty("tooltipText")
		if type(tip) == "string" then entry.tooltip_text = tip end
		if kind == "Dropdown" then
			for i, item in ipairs(control.list) do
				entry.choices[i] = type(item) == "table" and item.label or item
			end
			entry.selected = math.max(control.selIndex - 1, 0)
		elseif kind == "Slider" then
			entry.value = control.val
			entry.divisions = control.divCount
		elseif kind == "Check" then
			entry.checked = control.state and true or false
		elseif kind == "Edit" then
			entry.text = control.buf
		elseif kind == "List" then
			for i, item in ipairs(control.list) do
				entry.rows[i] = { key = rowKey(item, i), label = text(control:GetRowValue(1, i, item)) }
				if item == control.selValue then entry.row_selected = rowKey(item, i) end
			end
			if control.defaultText then entry.note = text(control:GetProperty("defaultText")) end
		end
		table.insert(controls, entry)
		if kind == "List" then
			for childName, child in pairs(control.controls) do
				add(name .. "." .. tostring(childName), child)
			end
		end
	end
	for key, control in pairs(popup.controls) do
		add(tostring(key), control)
	end
	table.sort(controls, function(a, b)
		if a.pos[2] ~= b.pos[2] then return a.pos[2] < b.pos[2] end
		if a.pos[1] ~= b.pos[1] then return a.pos[1] < b.pos[1] end
		return a.name < b.name
	end)
	return { kind = popupKind, title = text(popup:GetProperty("title")), controls = controls }
end

-- The tooltip a dropdown entry (`index`, 0-based) or a button of the open popup shows
function popupTooltip(name, index)
	local control = popupControl(name)
	if not (control and control.tooltipFunc) then return nil end
	local tt = new("Tooltip")
	if index then
		local entry = control.list[index + 1]
		if entry == nil then return nil end
		control.tooltipFunc(tt, "HOVER", index + 1, entry)
	else
		control.tooltipFunc(tt)
	end
	return tooltipDump(tt)
end

-- ---- Crafting an item ----

-- The 0-based index of the entry of a dropdown named by its `name` or its label without colour
-- codes
function entryIndex(control, name)
	for i, entry in ipairs(control.list) do
		local label = type(entry) == "table" and (entry.name or entry.label) or entry
		label = label:gsub("%^x%x%x%x%x%x%x", ""):gsub("%^%d", "")
		if label == name then return i - 1, type(entry) == "table" and entry.label or entry end
	end
	error("no entry " .. name)
end

-- The Craft window, as a user fills it in: rarity, name, type and base, then Create
function craftBase(rarity, title, typeName, baseName)
	act.open_popup("craftDisplayItem")
	local controls = main.popups[1].controls
	for _, step in ipairs({ { "rarity", rarity }, { "type", typeName } }) do
		local index, label = entryIndex(controls[step[1]], step[2])
		act.popup_set(step[1], { Choice = { index = index, label = label } })
	end
	if title then act.popup_set("title", { Text = title }) end
	-- choosing the type gave the base dropdown its list
	local index, label = entryIndex(controls.base, baseName)
	act.popup_set("base", { Choice = { index = index, label = label } })
	act.popup_press("save")
	assert(not main.popups[1], "Create left a window open")
end
