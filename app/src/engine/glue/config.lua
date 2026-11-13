-- The Config tab for the Rust host: the view, option changes, config sets, custom modifiers
-- and undo. Changes go through the controls' own callbacks, so undo, `BuildModList` and
-- `buildFlag` happen exactly as in the classic app.
-- Runs after bootstrap.lua and tooltip.lua. Errors use level 0.

glue = glue or {}

local function config_tab()
	return pob.build.configTab
end

-- PoB's option table, loaded once; `varData` for each option by var.
local options, option_by_var
local function option_list()
	if not options then
		options = LoadModule("Modules/ConfigOptions")
		option_by_var = {}
		for _, varData in ipairs(options) do
			if varData.var then option_by_var[varData.var] = varData end
		end
	end
	return options, option_by_var
end

-- The first value of `borderFunc` for a set option that is no longer shown (ConfigTab.lua).
local INVALID_BORDER = 0.753
local INVALID_TEXT = "This config option is conditional with missing source and is invalid."

local function strip(text)
	return (StripEscapes(tostring(text)))
end

local function set_ids(tab)
	local sets = {}
	for _, id in ipairs(tab.configSetOrderList) do
		sets[#sets + 1] = { id = id, title = tab.configSets[id].title or "Default", untitled = tab.configSets[id].title == nil }
	end
	return sets
end

-- What the option's tooltip shows now, as a recorder would see it.
local function tooltip_lines(tab, control, varData)
	local recorder = glue.tooltip()
	local ok
	if varData.tooltipFunc then
		local index = control.selIndex
		local value = control.list and index and control.list[index]
		ok = pcall(varData.tooltipFunc, recorder, "BODY", index, value)
	else
		local text
		ok, text = pcall(control.GetProperty, control, "tooltipText")
		if ok and text and text ~= "" then recorder:AddLine(14, text) end
	end
	local lines = {}
	if ok then
		for _, line in ipairs(recorder:lines()) do
			if line.text then
				lines[#lines + 1] = { size = line.size, text = strip(line.text) }
			else
				lines[#lines + 1] = line
			end
		end
	end
	return lines
end

local function option_view(tab, varData, control)
	local kind = varData.type
	local view = { var = varData.var, kind = kind }
	if kind == "countAllowZero" then
		view.kind = "count"
		view.allow_zero = true
	end
	local label = varData.label or varData.var
	if kind == "check" and control.label then
		label = control.label
		if type(label) == "function" then label = label() end
	end
	view.label = strip(label)
	view.shown = control:IsShown() and true or false
	-- `false` is a value, so don't use `or`
	local value = tab.input[varData.var]
	if type(value) == "boolean" or type(value) == "number" or type(value) == "string" then
		view.value = value
	end
	local placeholder = tab.placeholder[varData.var]
	if type(placeholder) == "number" or type(placeholder) == "string" then
		view.placeholder = placeholder
	end
	if kind == "list" then
		local choices = {}
		for _, item in ipairs(control.list) do
			choices[#choices + 1] = { label = strip(item.label or item.val), value = tostring(item.val) }
		end
		-- an empty Lua table is ambiguous to serde, so leave it out
		if #choices > 0 then view.choices = choices end
	end
	if kind == "list" then
		local item = control.list[control.selIndex]
		if value ~= nil and item and item.val == value then view.selected = tostring(value) end
	end
	-- PoB's border colours an option red when it is set but its condition isn't met; its label
	-- wrapper and tooltip only react like that unless the option is `hideIfInvalid`
	if not varData.hideIfInvalid and control.borderFunc then
		local red = control.borderFunc()
		view.invalid = red == INVALID_BORDER
	end
	local tooltip = tooltip_lines(tab, control, varData)
	if view.invalid then
		-- the wording of the line ConfigTab.lua appends to the control's tooltip
		tooltip[#tooltip + 1] = { size = 14, text = INVALID_TEXT }
	end
	if #tooltip > 0 then view.tooltip = tooltip end
	return view
end

-- The colour PoB's custom modifier box shows each line of `text` in while it isn't selected:
-- its own `inactiveText` (ConfigTab.lua:78-86), MAGIC for a line modLib.parseMod reads whole,
-- else UNSUPPORTED. One code a line of the text; PoB's extra empty line at the end is left out.
local function custom_mod_line_colours(block, text)
	local colours = {}
	local edit = block and block.controls.textEdit
	if not (edit and edit.inactiveText) then
		return colours
	end
	local lines = select(2, text:gsub("\n", "")) + 1
	for line in edit.inactiveText(text):gmatch("([^\n]*)\n") do
		if #colours == lines then
			break
		end
		colours[#colours + 1] = line:match("^(%^x%x%x%x%x%x%x)") or line:match("^(%^%d)") or ""
	end
	return colours
end

function glue.config_view()
	local tab = config_tab()
	local sections = {}
	local current
	for _, varData in ipairs((option_list())) do
		if varData.section then
			current = { name = varData.section, column = varData.col or 1, options = {} }
			sections[#sections + 1] = current
		elseif varData.var and current then
			local control = tab.varControls[varData.var]
			if control then
				current.options[#current.options + 1] = option_view(tab, varData, control)
			end
		end
	end
	local filled = {}
	for _, section in ipairs(sections) do
		if #section.options > 0 then filled[#filled + 1] = section end
	end
	local groups = {}
	local blocks = tab.customModsBlockControls or {}
	for i, block in ipairs(tab.configSets[tab.activeConfigSetId].customModsList or {}) do
		local text = block.text or ""
		groups[#groups + 1] = {
			title = block.title or "Default",
			enabled = block.enabled ~= false,
			text = text,
			line_colours = custom_mod_line_colours(blocks[i], text),
		}
	end
	return {
		sets = set_ids(tab),
		active_set = tab.activeConfigSetId,
		custom_mods = groups,
		sections = filled,
	}
end

local function bad_value(var, what)
	error(var .. " " .. what, 0)
end

local function finite(value)
	return type(value) == "number" and value == value and value ~= math.huge and value ~= -math.huge
end

local function number_text(value)
	return (string.format("%.14g", value))
end

function glue.set_config(var, value)
	local tab = config_tab()
	local control = tab.varControls[var]
	if not control then
		error("unknown config option " .. tostring(var), 0)
	end
	local _, by_var = option_list()
	local kind = by_var[var].type
	local current = tab.input[var]
	if kind == "check" then
		if value == nil then value = false end
		if type(value) ~= "boolean" then bad_value(var, "takes on or off") end
		if (current or false) == value then return end
		control.state = value
		control.changeFunc(value)
	elseif kind == "list" then
		if type(value) ~= "string" then bad_value(var, "takes one of its choices") end
		local index
		for i, item in ipairs(control.list) do
			if tostring(item.val) == value then index = i break end
		end
		if not index then bad_value(var, "has no choice '" .. value .. "'") end
		control:SetSel(index)
		if control.selIndex ~= index then bad_value(var, "couldn't select '" .. value .. "'") end
	elseif kind == "text" then
		if value == nil then value = "" end
		if type(value) ~= "string" then bad_value(var, "takes text") end
		if (current or "") == value and current ~= nil then return end
		control:SetText(value, true)
	elseif kind == "count" or kind == "countAllowZero" or kind == "integer" or kind == "float" then
		local buf
		if value == nil then
			-- an empty buffer leaves the option unset, so the placeholder applies
			buf = ""
			if current == nil then return end
		else
			if not finite(value) then bad_value(var, "takes a number") end
			if kind ~= "float" and value ~= math.floor(value) then bad_value(var, "takes a whole number") end
			if kind == "count" and value < 0 then bad_value(var, "can't be negative") end
			if current == value then return end
			buf = number_text(value)
		end
		control:SetText(buf, true)
	else
		bad_value(var, "can't be changed")
	end
end

local function config_set_index(tab, id)
	for index, setId in ipairs(tab.configSetOrderList) do
		if setId == id then return index end
	end
	error("no config set " .. tostring(id), 0)
end

function glue.select_config_set(id)
	local tab = config_tab()
	config_set_index(tab, id)
	if id == tab.activeConfigSetId then return end
	-- as the set selector does
	tab:SetActiveConfigSet(id)
	tab:AddUndoState()
end

-- What the manager's name box (ConfigSetListControl.lua:45) holds once `title` is typed into
-- it: the box's own filter and 100-byte limit, and Save needs a visible character
local function valid_title(title)
	if type(title) ~= "string" then
		error("a config set needs a name", 0)
	end
	local box = new("EditControl", nil, {0, 0, 0, 0}, "", nil, nil, 100)
	local text = title:gsub("\r", ""):gsub(box.filterPattern, "")
	if not text:match("%S") then
		error("a config set needs a name", 0)
	end
	if #text > box.limit then
		error("the text is too long", 0)
	end
	return text
end

-- As the manager's New button does, then switches to the new set.
function glue.new_config_set(title)
	title = valid_title(title)
	local tab = config_tab()
	local configSet = tab:NewConfigSet(nil, title)
	table.insert(tab.configSetOrderList, configSet.id)
	tab.modFlag = true
	tab:SetActiveConfigSet(configSet.id)
	tab:AddUndoState()
end

function glue.delete_config_set(id)
	local tab = config_tab()
	local index = config_set_index(tab, id)
	if #tab.configSetOrderList < 2 then
		error("can't delete the only config set", 0)
	end
	table.remove(tab.configSetOrderList, index)
	tab.configSets[id] = nil
	if id == tab.activeConfigSetId then
		tab:SetActiveConfigSet(tab.configSetOrderList[math.max(1, index - 1)])
	end
	tab:AddUndoState()
	tab.build:SyncLoadouts()
	tab.build.buildFlag = true
end

-- Custom modifier groups go through the group's own controls, like clicking in the tab.
local function group_controls(group)
	local tab = config_tab()
	local blocks = tab.customModsBlockControls
	if not blocks then error("this build has no custom modifiers", 0) end
	if type(group) ~= "number" or group < 1 or group > #blocks or group ~= math.floor(group) then
		error("no custom modifier group " .. tostring(group), 0)
	end
	return blocks[group].controls, tab.configSets[tab.activeConfigSetId].customModsList[group]
end

function glue.set_custom_mod_text(group, text)
	if type(text) ~= "string" then error("custom modifiers must be text", 0) end
	local controls, block = group_controls(group)
	if (block.text or "") == text then return end
	controls.textEdit:SetText(text, true)
end

function glue.set_custom_mod_title(group, title)
	if type(title) ~= "string" then error("a group title must be text", 0) end
	local controls, block = group_controls(group)
	if (block.title or "Default") == title then return end
	-- typed into its box, which drops what its filter rejects (ConfigTab.lua:35)
	glue.type_into(controls.titleEdit, title)
end

function glue.set_custom_mod_enabled(group, enabled)
	if type(enabled) ~= "boolean" then error("enabled must be on or off", 0) end
	local controls, block = group_controls(group)
	if (block.enabled ~= false) == enabled then return end
	controls.enableCheck.state = enabled
	controls.enableCheck.changeFunc(enabled)
end

function glue.add_custom_mod_group()
	local button = config_tab().controls.customModsAddBlock
	if not button then error("this build has no custom modifiers", 0) end
	button.onClick()
end

function glue.remove_custom_mod_group(group)
	local controls = group_controls(group)
	controls.deleteBtn.onClick()
end

local function undo_tab(tab)
	if tab ~= "config" then error("nothing to undo in " .. tostring(tab), 0) end
	return config_tab()
end

function glue.undo(tab)
	local configTab = undo_tab(tab)
	configTab:Undo()
	configTab:BuildModList()
	configTab.build.buildFlag = true
end

function glue.redo(tab)
	local configTab = undo_tab(tab)
	configTab:Redo()
	configTab:BuildModList()
	configTab.build.buildFlag = true
end

-- ---- "What if" tooltips, set copy and reorder (R22, NF-4) ----

-- What hovering an option's control shows: its `tooltipFunc` as PoB wraps it (ConfigTab.lua:
-- 700-764, the "Toggling/Selecting this option will give you:" comparison), called with what
-- the control's own draw passes: a check box its state (CheckBoxControl.lua:80), a dropdown's
-- box "BODY" and its selection (DropDownControl.lua:295-298), an edit box nothing. An option
-- whose control has no `tooltipFunc` shows its `tooltipText`, as TooltipHost does. Nil when the
-- option has no control, or PoB doesn't show it.
function glue.config_hover(var)
	local tab = config_tab()
	local control = tab.varControls[var]
	if not (control and control:IsShown()) then
		return nil
	end
	local tooltip = new("Tooltip")
	if control.tooltipFunc then
		if control._className == "CheckBoxControl" then
			control.tooltipFunc(tooltip, control.state)
		elseif control._className == "DropDownControl" then
			control.tooltipFunc(tooltip, "BODY", control.selIndex, control.list[control.selIndex])
		else
			control.tooltipFunc(tooltip)
		end
	else
		local text = control:GetProperty("tooltipText")
		if text and text ~= "" then
			tooltip:AddLine(14, text)
		end
	end
	return glue.tooltip_lines(tooltip)
end

-- What hovering a custom modifier group's enable box shows (ConfigTab.lua:53-67)
function glue.custom_mod_hover(group)
	local blocks = config_tab().customModsBlockControls
	local block = blocks and blocks[group]
	if not block then
		return nil
	end
	local tooltip = new("Tooltip")
	block.controls.enableCheck.tooltipFunc(tooltip)
	return glue.tooltip_lines(tooltip)
end

local function with_config_set_list(fn)
	glue.with_set_list("config set", function()
		config_tab():OpenConfigSetManagePopup()
	end, fn)
end

-- The manager's Copy button and its name popup (ConfigSetListControl.lua:13-25, 44-69): the copy
-- is added at the end of the list, not switched to
function glue.copy_config_set(id, title)
	local tab = config_tab()
	local index = config_set_index(tab, id)
	glue.check_title("config set", title)
	with_config_set_list(function(list)
		list.selIndex, list.selValue = index, id
		list.controls.copy.onClick()
		glue.save_name("config set", title)
	end)
end

-- The manager's Rename button and its name popup (ConfigSetListControl.lua:32-34, 44-69): the
-- box opens on the set's title and takes the new text as an edit of it
function glue.rename_config_set(id, title)
	local tab = config_tab()
	local index = config_set_index(tab, id)
	glue.check_title("config set", title)
	with_config_set_list(function(list)
		list.selIndex, list.selValue = index, id
		list.controls.rename.onClick()
		glue.save_name("config set", title)
	end)
end

-- Dragging a row of the manager's list (ConfigSetListControl.lua:79-81 only sets modFlag)
function glue.move_config_set(from, to)
	local order = config_tab().configSetOrderList
	if not (order[from] and order[to]) then
		error("no such config set", 0)
	end
	if from == to then
		return
	end
	with_config_set_list(function(list)
		glue.drop_list_row(list, from, to)
	end)
	glue.frame()
end
