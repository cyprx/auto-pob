-- Popup sessions: PoB's own popups (Enchant, Anoint, Corrupt, the custom-modifier windows)
-- driven headless (R1-R7).
--
-- The open session's PopupDialog lives here, not in `main.popups`: PoB draws `popups[1]` in
-- every frame and blanks its other tooltips while one exists. `with_session` puts it at
-- index 1 for the length of one command, a view or a tooltip, and takes it out again, so
-- between calls `#main.popups == 0`.

glue = glue or {}

local NOT_SHOWN = "PoB doesn't show that control for this item"

-- { popup, kind, owner, item, build }: nil while no session is open. The owner (R19) says what
-- ends it: `item` (the displayed item or the build changed), `build` (the build changed) or
-- `main` (nothing but PoB or a command; it needs no build and survives build changes).
local session

local function main()
	return pob.main.main
end

local function items_tab()
	return pob.build.itemsTab
end

-- ---- The session ----

-- A session whose owner is gone is over: an item session's item is no longer displayed or
-- its build was replaced; a build session's build was replaced; a main session has no owner
-- to lose (R19)
local function live()
	if not session then
		return false
	end
	if session.owner == "main" then
		return true
	end
	if session.build ~= pob.build then
		session = nil
		return false
	end
	if session.owner == "item" and session.item ~= pob.build.itemsTab.displayItem then
		session = nil
		return false
	end
	return true
end

-- Whether a session is open (the affix tooltips PoB blanks while a popup exists ask)
function glue.popup_live()
	return live()
end

-- Removes every popup above `depth` from the stack, newest first
local function close_above(depth)
	local popups = main().popups
	while #popups > depth do
		table.remove(popups, 1)
	end
end

-- Takes stock of the stack after something ran with `mine` (or nothing) in it. Returns true
-- when the session is over. A press that left a different popup (Craft's Create closes itself
-- and opens the item text editor) hands the session to that one (R2).
local function settle(mine, depth)
	local popups = main().popups
	local found, top
	for i = 1, #popups - depth do
		if popups[i] == mine then
			found = true
		end
	end
	top = popups[1]
	local count = #popups - depth
	if mine and found then
		close_above(depth)
		return false
	end
	if count > 0 then
		close_above(depth)
		session = session or {}
		session.popup = top
		-- only an item session's follow-up is the item text editor; any other keeps the
		-- opener's kind (Options' DPI dropdown closes the window and opens it again)
		if session.owner == nil or session.owner == "item" then
			session.kind = "EditText"
			session.owner = "item"
			session.item = items_tab().displayItem
			session.build = pob.build
		end
		return false
	end
	session = nil
	return true
end

-- Runs `fn` with the session's popup at `main.popups[1]`. Returns whether the session ended
-- during the call. An error from `fn` is raised after the stack is settled.
local function with_session(fn)
	local popups = main().popups
	local depth = #popups
	local mine = session.popup
	table.insert(popups, 1, mine)
	local ok, err = pcall(fn)
	local ended = settle(mine, depth)
	if not ok then
		error(err, 0)
	end
	return ended
end

-- Ends the session; true when one was open (the engine then sends `Popup(None)`)
function glue.popup_end(keep_main)
	-- a main-owned session survives a build change (R19)
	if keep_main and session ~= nil and session.owner == "main" then
		return false
	end
	-- not `live()`: the command that just ran may itself have replaced the item (R4)
	local was = session ~= nil
	-- an Options window that goes away unanswered is cancelled, so PoB's half-edited values
	-- don't stay in `main` for the next save to write
	if was and session.kind == "Options" then
		glue.popup_cancel_options()
	end
	session = nil
	return was
end

-- Whether a save PoB made by itself cancelled the Options window since the last call
local lost = false
function glue.popup_take_lost()
	local was = lost
	lost = false
	return was
end

-- Whether the Options window's own button is being pressed (its Save writes the settings)
function glue.popup_options_pressing()
	return session ~= nil and session.kind == "Options" and session.pressing == true
end

-- A save PoB makes by itself while Options is open (a token, Shutdown) would write the window's
-- unsaved edits: the window is cancelled first (R29)
function glue.popup_cancel_for_save()
	if glue.popup_is_options() and not glue.popup_options_pressing() then
		glue.popup_cancel_options()
		session = nil
		lost = true
	end
end

-- Whether the open session is PoB's Options window
function glue.popup_is_options()
	return session ~= nil and session.kind == "Options"
end

-- ---- Opening ----

-- the panel's own button for each opener (R3)
local OPENERS = {
	craft = function() return "craftDisplayItem" end,
	enchant = function(second) return second and "displayItemEnchant2" or "displayItemEnchant" end,
	anoint = function(slot)
		return slot == 1 and "displayItemAnoint" or ("displayItemAnoint" .. slot)
	end,
	corrupt = function() return "displayItemCorrupt" end,
	add_modifier = function() return "displayItemAddCustom" end,
	add_implicit = function() return "displayItemAddImplicit" end,
	add_crucible = function() return "displayItemAddCrucible" end,
}

-- the kinds the glue reports, in the serde shape of `PopupKind`
local KINDS = {
	craft = function() return "Craft" end,
	enchant = function(second) return { Enchant = { second = second and true or false } } end,
	anoint = function(slot) return { Anoint = { slot = slot - 1 } } end,
	corrupt = function() return "Corrupt" end,
	add_modifier = function() return "AddModifier" end,
	add_implicit = function() return "AddImplicit" end,
	add_crucible = function() return "AddCrucible" end,
}

-- The windows the display panel offers, in the panel's order: its button and the kind it opens
local PANEL = {
	{ "displayItemEnchant", { Enchant = { second = false } } },
	{ "displayItemEnchant2", { Enchant = { second = true } } },
	{ "displayItemAnoint", { Anoint = { slot = 0 } } },
	{ "displayItemAnoint2", { Anoint = { slot = 1 } } },
	{ "displayItemAnoint3", { Anoint = { slot = 2 } } },
	{ "displayItemAnoint4", { Anoint = { slot = 3 } } },
	{ "displayItemCorrupt", "Corrupt" },
	{ "displayItemAddImplicit", "AddImplicit" },
	{ "displayItemAddCustom", "AddModifier" },
	{ "displayItemAddCrucible", "AddCrucible" },
}

-- The kinds of the shown openers (`DisplayItemView.popups`, R3)
function glue.panel_popups(tab)
	local kinds = {}
	for _, entry in ipairs(PANEL) do
		local control = tab.controls[entry[1]]
		if control and control:IsShown() and control:IsEnabled() then
			table.insert(kinds, entry[2])
		end
	end
	return kinds
end

-- Presses the panel's button for `kind` (`arg` is the second flag or the 1-based anoint
-- slot) and keeps the popup it opens as the session.
function glue.popup_open(kind, arg)
	local open
	if kind == "options" then
		-- a window opened over another Options window would take its edits as the state to
		-- go back to
		glue.popup_cancel_options()
		-- PoB's Save writes `main.connectionProtocol` into `launch`, but only the window's
		-- dropdown ever sets it: start from what is stored, on every open
		main().connectionProtocol = pob.main.connectionProtocol or 0
		-- PoB's own opener; the window needs no build (R13)
		local popups = main().popups
		local depth = #popups
		local ok, err = pcall(function()
			main():OpenOptionsPopup()
		end)
		local count = #popups - depth
		local top = popups[1]
		close_above(depth)
		if not ok then
			error(err, 0)
		end
		if count == 0 then
			error("PoB didn't open that window", 0)
		end
		session = { popup = top, kind = "Options", owner = "main" }
		return
	end
	if kind == "node_replacement" then
		-- the right-click's body (PassiveTreeView.lua:519-533) without `buildFlag`, which
		-- changes nothing: the node must pass PoB's rule; an allocated mastery is the effect
		-- picker's, not this window's
		local node = pob.build.spec.nodes[arg]
		if not (node and (glue.node_replaceable(node)
			or (glue.node_mastery_effects(node) and not node.alloc))) then
			error("PoB doesn't offer that node for replacement", 0)
		end
		open = function()
			pob.build.treeTab:ModifyNodePopup(node)
		end
	else
		local name = OPENERS[kind]
		if not name then
			error(NOT_SHOWN, 0)
		end
		local control = items_tab().controls[name(arg)]
		if not (control and control:IsShown() and control:IsEnabled()) then
			error(NOT_SHOWN, 0)
		end
		open = control.onClick
	end
	glue.popup_cancel_options()
	local tab = items_tab()
	local popups = main().popups
	local depth = #popups
	local ok, err = pcall(open)
	local count = #popups - depth
	local top = popups[1]
	close_above(depth)
	if not ok then
		if kind == "node_replacement" then
			-- PoB builds the window from the tattoos that fit the node and raises when none do
			error("PoB offers no modifier for that node (" .. tostring(err) .. ")", 0)
		end
		error(err, 0)
	end
	if count == 0 then
		error("PoB didn't open that window", 0)
	end
	-- a failed open above left the session it found alone; this one replaces it (R4)
	local shown = kind == "node_replacement" and { NodeReplacement = { node = arg } } or KINDS[kind](arg)
	session = { popup = top, kind = shown, owner = "item", item = tab.displayItem, build = pob.build }
end

-- ---- Controls ----

-- whether the control's class is, or inherits from, `name`
local function is_a(control, name)
	local function walk(class)
		if class._className == name then
			return true
		end
		for _, parent in ipairs(rawget(class, "_parents") or {}) do
			if walk(parent) then
				return true
			end
		end
		return false
	end
	local class = getmetatable(control)
	return class ~= nil and walk(class)
end

local KIND_OF = {
	{ "LabelControl", "Label" },
	{ "ButtonControl", "Button" },
	{ "DropDownControl", "Dropdown" },
	{ "SliderControl", "Slider" },
	{ "CheckBoxControl", "Check" },
	{ "EditControl", "Edit" },
	{ "ListControl", "List" },
}

local function kind_of(control)
	for _, entry in ipairs(KIND_OF) do
		if is_a(control, entry[1]) then
			return entry[2]
		end
	end
end

-- Every control the popup shows, by name: the popup's own controls and a list's children
-- (`notableDB.search`). Numeric keys are named by their number.
local function all_controls(popup)
	local found = {}
	local options = session ~= nil and session.kind == "Options" and session.popup == popup
	for key, control in pairs(popup.controls) do
		local name = tostring(key)
		local kind = kind_of(control)
		-- the rows the app doesn't offer are not in the window at all (R13)
		if kind and not (options and glue.settings_hidden(name)) then
			found[name] = { control = control, kind = kind }
			if kind == "List" and control.controls then
				for child_key, child in pairs(control.controls) do
					local child_kind = kind_of(child)
					if child_kind then
						found[name .. "." .. tostring(child_key)] = { control = child, kind = child_kind }
					end
				end
			end
		end
	end
	return found
end

-- The control a command names; the session's popup must be at index 1
local function find(name)
	local entry = all_controls(session.popup)[name]
	if not entry then
		if glue.popup_is_options() and glue.settings_hidden(name) then
			error("PoB doesn't offer that here", 0)
		end
		error("no such control", 0)
	end
	return entry.control, entry.kind
end

local function row_key(value, index)
	if type(value) == "table" then
		return tostring(value.id or index)
	end
	return tostring(value)
end

local function text_of(value)
	if type(value) == "string" then
		return value
	end
	return ""
end

-- one control as the view shows it
local function control_view(name, control, kind, popup_x, popup_y)
	local x, y = control:GetPos()
	local width, height = control:GetSize()
	local view = {
		name = name,
		kind = kind,
		label = text_of(control:GetProperty("label")),
		pos = { x - popup_x, y - popup_y },
		size = { width or 0, height or 0 },
		shown = control:IsShown() and true or false,
		enabled = control:IsEnabled() and true or false,
		choices = {},
		selected = 0,
		value = 0,
		checked = false,
		text = "",
		hover_tooltip = kind ~= "List" and control.tooltipFunc ~= nil,
		rows = {},
	}
	local tooltip_text = control:GetProperty("tooltipText")
	if type(tooltip_text) == "string" then
		view.tooltip_text = tooltip_text
	end
	if kind == "Dropdown" then
		local drop = glue.drop_list(control)
		view.choices = drop.labels
		view.selected = drop.selected
	elseif kind == "Slider" then
		view.value = control.val or 0
		view.divisions = control.divCount
	elseif kind == "Check" then
		view.checked = control.state and true or false
	elseif kind == "Edit" then
		view.text = control.buf or ""
	elseif kind == "List" then
		glue.list_sync(control)
		for i, value in ipairs(control.list) do
			local ok, label = pcall(control.GetRowValue, control, 1, i, value)
			view.rows[i] = { key = row_key(value, i), label = ok and text_of(label) or "" }
		end
		if control.selValue ~= nil then
			for i, value in ipairs(control.list) do
				if value == control.selValue then
					view.row_selected = row_key(value, i)
				end
			end
		end
		view.note = control.defaultText and text_of(control:GetProperty("defaultText")) or nil
	end
	return view
end

-- The open popup as a `PopupView`, or nil without a session. Controls are ordered by
-- position, then name, because the popup keeps them in a hash table.
function glue.popup_view()
	if not live() then
		return nil
	end
	local view
	with_session(function()
		local popup = session.popup
		local popup_x, popup_y = popup:GetPos()
		view = { kind = session.kind, title = text_of(popup:GetProperty("title")), controls = {} }
		for name, entry in pairs(all_controls(popup)) do
			table.insert(view.controls, control_view(name, entry.control, entry.kind, popup_x, popup_y))
		end
		table.sort(view.controls, function(a, b)
			if a.pos[2] ~= b.pos[2] then
				return a.pos[2] < b.pos[2]
			elseif a.pos[1] ~= b.pos[1] then
				return a.pos[1] < b.pos[1]
			end
			return a.name < b.name
		end)
	end)
	return view
end

-- ---- Setting and pressing ----

local function require_session()
	if not live() then
		error("no popup is open", 0)
	end
end

local function require_usable(control)
	if not (control:IsShown() and control:IsEnabled()) then
		error(NOT_SHOWN, 0)
	end
end

-- Sets `control` (of `control_kind`, from KIND_OF) as the user would. `kind` is the input's
-- kind ("choice", "number", "check", "text" or "row"); a choice carries the label that was
-- shown (R8). For the other glue files too (Compare's controls, R6).
function glue.set_control(control, control_kind, kind, value, label)
	if kind == "choice" and control_kind == "Dropdown" then
		local entry = control.list[value + 1]
		if entry == nil then
			error("no such choice", 0)
		end
		if (type(entry) == "table" and entry.label or entry) ~= label then
			error("the list changed; pick again", 0)
		end
		control:SetSel(value + 1)
	elseif kind == "number" and control_kind == "Slider" then
		control:SetVal(value)
	elseif kind == "check" and control_kind == "Check" then
		if (control.state and true or false) ~= value then
			glue.press_check(control, value)
		end
	elseif kind == "text" and control_kind == "Edit" then
		glue.paste_into(control, value)
	elseif kind == "row" and control_kind == "List" then
		glue.list_sync(control)
		for i, entry in ipairs(control.list) do
			if row_key(entry, i) == value then
				control:SelectIndex(i)
				control:OnSelClick(i, entry, false)
				return
			end
		end
		error("no such choice", 0)
	else
		error("no such control", 0)
	end
end

-- A control outside a popup as the views show it (`PopupControl`), positions as `GetPos()`
-- gives them; nil for a kind the views don't have
function glue.describe_control(name, control)
	local kind = kind_of(control)
	return kind and control_view(name, control, kind, 0, 0), kind
end

-- Sets a control as the user would; see `glue.set_control`.
function glue.popup_set(name, kind, value, label)
	require_session()
	with_session(function()
		local control, control_kind = find(name)
		require_usable(control)
		glue.set_control(control, control_kind, kind, value, label)
	end)
end

-- Presses a button. Returns whether the session ended and whether PoB asked for a
-- recalculation (`buildFlag`, which a tattoo sets).
function glue.popup_press(name)
	require_session()
	local flag_before = pob.build.buildFlag and true or false
	local options = session.kind == "Options"
	session.pressing = true
	local ok, ended = pcall(with_session, function()
		local control, kind = find(name)
		if kind ~= "Button" then
			error("no such control", 0)
		end
		require_usable(control)
		control.onClick()
	end)
	if session then
		session.pressing = nil
	end
	if not ok then
		error(ended, 0)
	end
	if options then
		-- a Cancel put the shared values back over an override
		glue.settings_reapply()
	end
	return { ended = ended, build_flag = (pob.build.buildFlag and not flag_before) and true or false }
end

-- Presses Options' Cancel (which restores the values the window changed as it went)
function glue.popup_cancel_options()
	if not glue.popup_is_options() then
		return
	end
	local cancel = session.popup.controls.cancel
	if cancel then
		pcall(with_session, function()
			cancel.onClick()
		end)
	end
	-- Cancel puts back what the window opened with, an override included
	glue.settings_reapply()
end

-- Cancel and Escape: PoB only closes the popup. Escape on Options is PoB's `cancel` button.
function glue.popup_close()
	glue.popup_cancel_options()
	session = nil
end

-- The tooltip of a dropdown entry (`index`, 0-based) or of a button, as PoB would build it on
-- hover. Nil when the control or the entry is gone.
function glue.popup_tooltip(name, index)
	if not live() then
		return nil
	end
	local out
	with_session(function()
		local entry = all_controls(session.popup)[name]
		local control = entry and entry.control
		if not (control and control.tooltipFunc and control:IsShown()) then
			return
		end
		local tooltip = new("Tooltip")
		if index then
			local value = control.list and control.list[index + 1]
			if value == nil then
				return
			end
			control.tooltipFunc(tooltip, "HOVER", index + 1, value)
		else
			control.tooltipFunc(tooltip)
		end
		out = glue.read_tooltip(tooltip)
	end)
	return out
end

-- ---- The anoint list's stat sort (R17) ----

-- The open Anoint session's notable list, or nil
local function notable_list()
	if not live() or type(session.kind) ~= "table" or not session.kind.Anoint then
		return nil
	end
	return session.popup.controls.notableDB
end

-- Whether the Anoint window's list has a stat sort to run: PoB's Draw preamble would restart
-- it on a new flag or output revision, and a live builder is work
function glue.anoint_sort_pending()
	local control = notable_list()
	if not (control and control.ListBuilder) then
		return false
	end
	if not (control.sortDetail and control.sortDetail.stat) then
		return false
	end
	return glue.list_pending(control)
end

-- Starts a failed sort again, as `SetSortMode` does: a failed resume dropped the builder, and
-- picking the sort that is already selected runs no PoB code (Retry)
function glue.anoint_sort_restart()
	local control = notable_list()
	if control then
		control.listBuildFlag = true
	end
end

-- One slice of the stat sort. Returns `{ finished, percent }`; a failed resume is an error.
function glue.anoint_sort_step()
	local control = notable_list()
	if not control then
		return { finished = true }
	end
	local result
	with_session(function()
		result = glue.list_step(control)
	end)
	return result
end
