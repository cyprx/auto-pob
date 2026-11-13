-- Popup sessions: PoB's own popups (Enchant, Anoint, Corrupt, the custom-modifier windows)
-- driven headless (R1-R7).
--
-- The open session's PopupDialog lives here, not in `main.popups`: PoB draws `popups[1]` in
-- every frame and blanks its other tooltips while one exists. `with_session` puts it at
-- index 1 for the length of one command, a view or a tooltip, and takes it out again, so
-- between calls `#main.popups == 0`.

glue = glue or {}

local NOT_SHOWN = "PoB doesn't show that control for this item"

-- { popup, kind, owner, item, gen }: nil while no session is open (`gen`: the build's
-- `glue.build_generation()` when it opened). The owner (R19) says what
-- ends it: `item` (the displayed item or the build changed), `build` (the build changed) or
-- `main` (nothing but PoB or a command; it needs no build and survives build changes).
local session

-- Ends the session; a timeless search in it ends too (R17)
local function drop()
	session = nil
	if glue.timeless_cancel then
		glue.timeless_cancel()
	end
end

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
	-- the Game Version window stands over the new build its open left; another build ends it
	if session.kind == "GameVersion" and session.gen ~= glue.build_generation() then
		drop()
		return false
	end
	if session.owner == "main" then
		return true
	end
	if session.gen ~= glue.build_generation() then
		drop()
		return false
	end
	if session.owner == "item" and session.item ~= pob.build.itemsTab.displayItem then
		drop()
		return false
	end
	-- a session over something PoB's modal popup would have kept still (the Mod Browser's
	-- custom modifier group) is over once that thing is gone (R28)
	if session.still_valid and not session.still_valid() then
		drop()
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
			session.gen = glue.build_generation()
		end
		return false
	end
	drop()
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
	-- a main-owned session survives a build change (R19); the Game Version window then ends
	-- in `live()`
	if keep_main and session ~= nil and session.owner == "main" then
		return false
	end
	-- a build-owned session ends with its build only (R19): a command that keeps the build
	-- leaves it, and a timeless search in it, running
	if not keep_main and session ~= nil and session.owner == "build" and session.gen == glue.build_generation()
		and not (session.still_valid and not session.still_valid()) then
		return false
	end
	-- not `live()`: the command that just ran may itself have replaced the item (R4)
	local was = session ~= nil
	-- an Options window that goes away unanswered is cancelled, so PoB's half-edited values
	-- don't stay in `main` for the next save to write
	if was and session.kind == "Options" then
		glue.popup_cancel_options()
	end
	drop()
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
		drop()
		lost = true
	end
end

-- PoB's "Game Version" window (Build.lua:1323-1350), which a build made for another game
-- version opens instead of loading (`enter_build`): a main-owned session, so the open that
-- made it doesn't end it; the next build does. `convert` is the Init arguments its Convert
-- button reopens the build with. Any way it ends but Convert is its Cancel, which leaves the new
-- build behind it.
function glue.popup_adopt_game_version(popup, convert)
	glue.popup_cancel_options()
	drop()
	session = { popup = popup, kind = "GameVersion", owner = "main", convert = convert, gen = glue.build_generation() }
end

-- Presses the Game Version window's button. PoB's Convert closes the window and runs Init
-- again with the conversion flag, outside a frame (Build.lua:1340-1343); here the build is
-- entered as an open does, so a frame initialises and calculates it. PoB's Cancel goes back to
-- the build list (Build.lua:1344-1347); here the new build behind the window stays.
local function game_version_press(name)
	if name ~= "convert" and name ~= "cancel" then
		error("no such control", 0)
	end
	local convert = session.convert
	drop()
	if name == "convert" then
		glue.enter_build(unpack(convert, 1, 4))
		return { ended = true, build_flag = false, rebuilt = true }
	end
	return { ended = true, build_flag = false }
end

-- Whether the open session is PoB's Options window
function glue.popup_is_options()
	return session ~= nil and session.kind == "Options"
end

-- ---- Trade links (R10, R20) ----

-- The app's trade realm and league (`SetTradeDefaults`): what PoB's league fetch would have
-- offered, since that fetch is refused
local trade = { realm = "PC", league = "Standard" }

function glue.trade_defaults(realm, league)
	trade.realm = realm
	trade.league = league
end

-- PoB's leagues that every realm has, in the order its fetch callback lists them
-- (CompareBuySimilar.lua:333-336)
local FIXED_LEAGUES = { "Standard", "Hardcore", "Ruthless", "Hardcore Ruthless" }

-- The league list PoB's fetch would build from the app's league, and that league's index. A
-- blank league is Standard.
function glue.trade_leagues()
	local league = trade.league
	if league == nil or league == "" then
		league = "Standard"
	end
	local list = {}
	if not isValueInArray(FIXED_LEAGUES, league) then
		table.insert(list, league)
	end
	for _, name in ipairs(FIXED_LEAGUES) do
		table.insert(list, name)
	end
	return list, isValueInArray(list, league)
end

function glue.trade_realm()
	return trade.realm
end

local TRADE_URL = "^https://www%.pathofexile%.com/trade/search/"

-- Runs `fn` with PoB's `OpenURL` recording a trade search link, and returns the last one. Any
-- other link is dropped; nothing is opened (the engine never opens a browser, R20).
function glue.capture_url(fn)
	local saved = OpenURL
	local url
	OpenURL = function(link)
		if type(link) == "string" and link:match(TRADE_URL) then
			url = link
		end
	end
	local ok, err = pcall(fn)
	OpenURL = saved
	if not ok then
		error(err, 0)
	end
	return url
end

-- Runs `fn` with every subscript launch refused, as inside `Build:Init` (R4): the trade
-- windows' league fetch never reaches the network
local function no_launches(fn)
	__net_in_init(true)
	local ok, err = pcall(fn)
	__net_in_init(false)
	if not ok then
		error(err, 0)
	end
	return err
end

-- `CompareBuySimilar`, loaded on first use and kept: its top level builds the trade stat table
-- (about 130 ms). It is our own instance, so its remembered realm and league are ours (R10).
local buy_similar

local function buy_module()
	if not buy_similar then
		buy_similar = LoadModule("Classes/CompareBuySimilar")
	end
	return buy_similar
end

-- The item and slot name PoB passes to `openPopup` for a `BuySource`
local function buy_target(source)
	local tab = items_tab()
	if source == "DisplayItem" then
		-- the panel's button (ItemsTab.lua:342-350)
		local control = tab.controls.displayItemBuySimilar
		if not (tab.displayItem and control:IsShown()) then
			error("PoB doesn't offer that here", 0)
		end
		return tab.displayItem, tab:GetComparisonSlotNameForItem(tab.displayItem)
	elseif type(source) == "table" and source.Slot then
		local slot = tab.slots[source.Slot]
		local item = slot and slot:IsShown() and tab.items[slot.selItemId]
		if not item then
			error("PoB doesn't offer that here", 0)
		end
		return item, source.Slot
	elseif type(source) == "table" and source.CompareSlot then
		return glue.compare_buy_item(source.CompareSlot), source.CompareSlot
	end
	error("PoB doesn't offer that here", 0)
end

-- The leagues PoB's fetch would have set (its callback, CompareBuySimilar.lua:323-339), from
-- the app's league; the pick is PoB's own selection callback, which rebuilds the URL. The list
-- starts as "Loading..." at index 1, so a `SetSel` to index 1 would run nothing.
-- (Its `selFunc` also records `M.lastLeagueByRealm`, as a user's pick does; only the fetch
-- callback reads that, and it never runs.)
local function buy_leagues(popup)
	local leagues = popup.controls.leagueDrop
	local list, index = glue.trade_leagues()
	leagues:SetList(list)
	leagues.selIndex = index
	leagues.selFunc(index, list[index])
end

-- The app's realm, picked as the user would; a change refetches the leagues (refused)
local function buy_realm(popup)
	local realms = popup.controls.realmDrop
	realms:SetSel(isValueInArray(realms.list, trade.realm) or 1)
end

local function is_timeless(s)
	return s ~= nil and s.kind == "TimelessJewel"
end

-- The trade windows: Buy similar (R10) and the timeless search's trade button (R17)
local function is_trade(s)
	return s ~= nil and (is_timeless(s) or (type(s.kind) == "table" and s.kind.BuySimilar ~= nil))
end

-- The timeless window's realms, as PoB lists them ("PC", "Sony", "Xbox"), by the app's realm
local TIMELESS_REALM = { PC = 1, PS4 = 2, Xbox = 3 }

-- The leagues PoB would have fetched for every realm, so the window never fetches (R17): its
-- cache `tradeLeaguesList` is filled from the app's league, which is also the pick. PoB's
-- dropdown takes a cached list as its own and wipes it on the next `SetList`, so each realm
-- gets a copy, made again before every realm pick.
local function timeless_leagues(tab)
	local list, index = glue.trade_leagues()
	for _, realm in ipairs({ "pc", "sony", "xbox" }) do
		tab.tradeLeaguesList[realm] = copyTable(list)
	end
	tab.timelessJewelLeagueSelect = list[index]
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
		drop()
		session = { popup = top, kind = "Options", owner = "main" }
		return
	end
	if kind == "buy_similar" then
		glue.popup_cancel_options()
		local item, slot_name = buy_target(arg)
		local module = buy_module()
		local popups = main().popups
		local depth = #popups
		local ok, err = pcall(no_launches, function()
			module.openPopup(item, slot_name, pob.build)
			if #popups > depth then
				buy_realm(popups[1])
				buy_leagues(popups[1])
			end
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
		drop()
		session = { popup = top, kind = { BuySimilar = { source = arg } }, owner = "build", gen = glue.build_generation() }
		return
	end
	if kind == "timeless" then
		glue.popup_cancel_options()
		local tab = pob.build.treeTab
		timeless_leagues(tab)
		-- the window opens on PC and the app's realm is picked in it, as a user would: opened on
		-- a remembered realm, PoB sets that realm's cached list twice, and the second `SetList`
		-- wipes it (TreeTab.lua:2376-2384, DropDownControl.lua:524-530)
		tab.selectedRealmIndex = 1
		local popups = main().popups
		local depth = #popups
		local ok, err = pcall(no_launches, function()
			tab:FindTimelessJewel()
			if #popups > depth then
				popups[1].controls.realmSelection:SetSel(TIMELESS_REALM[glue.trade_realm()] or 1)
			end
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
		drop()
		session = { popup = top, kind = "TimelessJewel", owner = "build", gen = glue.build_generation() }
		return
	end
	if kind == "spectre_library" then
		glue.popup_cancel_options()
		-- PoB's own buttons (Build.lua:552, CalcsTab.lua:105-109): the sidebar's, else the
		-- Calcs selector's; their main skills can differ, so each is refreshed as PoB draws it
		local button = glue.sidebar_spectre_library() or glue.calcs_spectre_library()
		if not (button and button:IsEnabled()) then
			error("PoB doesn't offer that here", 0)
		end
		local popups = main().popups
		local depth = #popups
		local ok, err = pcall(button.onClick)
		local count = #popups - depth
		local top = popups[1]
		close_above(depth)
		if not ok then
			error(err, 0)
		end
		if count == 0 then
			error("PoB didn't open that window", 0)
		end
		drop()
		session = { popup = top, kind = "SpectreLibrary", owner = "build", gen = glue.build_generation() }
		return
	end
	if kind == "mod_browser" then
		glue.popup_cancel_options()
		-- the group's own "Add Mod" button (ConfigTab.lua:42-44); `arg` is 1-based
		local tab = pob.build.configTab
		local blocks = tab.customModsBlockControls
		local block = blocks and blocks[arg]
		local list = tab.configSets[tab.activeConfigSetId].customModsList
		local data = list and list[arg]
		if not (block and data and block.blockData == data) then
			error("no custom modifier group " .. tostring(arg), 0)
		end
		local button = block.controls.addModBtn
		if not (button:IsShown() and button:IsEnabled()) then
			error("PoB doesn't offer that here", 0)
		end
		local popups = main().popups
		local depth = #popups
		local ok, err = pcall(button.onClick)
		local count = #popups - depth
		local top = popups[1]
		close_above(depth)
		if not ok then
			error(err, 0)
		end
		if count == 0 then
			error("PoB didn't open that window", 0)
		end
		drop()
		-- the window's Add writes into the group's table it was opened with: an undo, a set
		-- switch or a removed group replaces that table, which PoB's modal popup never lets
		-- happen, so the session ends then
		local function still_valid()
			local now = pob.build.configTab
			local set = now.configSets[now.activeConfigSetId]
			return now == tab and set ~= nil and set.customModsList ~= nil and set.customModsList[arg] == data
		end
		session = {
			popup = top,
			kind = { ModBrowser = { group = arg - 1 } },
			owner = "build",
			gen = glue.build_generation(),
			still_valid = still_valid,
		}
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
	drop()
	session = { popup = top, kind = shown, owner = "item", item = tab.displayItem, gen = glue.build_generation() }
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
		-- a list's rows are asked for one by one (`glue.popup_tooltip` with an index)
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
		view.ascii_only = control.filter == "^%w%p "
		-- a "%D" box: digits only, with + and - and stepping (EditControl.lua:64-75)
		view.digits = control.filter == "%D"
		view.limit = control.limit
	elseif kind == "List" then
		glue.list_sync(control)
		-- the rows as two flat lists: a table per row made the Mod Browser's 6188 rows cost
		-- 12k tables per view, and the garbage collection they caused (R28).
		-- `glue.describe_control` turns them into `rows`.
		local keys, labels = {}, {}
		for i, value in ipairs(control.list) do
			local ok, label = pcall(control.GetRowValue, control, 1, i, value)
			keys[i] = row_key(value, i)
			labels[i] = ok and text_of(label) or ""
		end
		view.row_keys, view.row_labels = keys, labels
		-- the selected row is `selIndex`: a Shift+click moves `selValue` but not the
		-- selection (ListControl.lua:76-85)
		local sel = control.selIndex and control.list[control.selIndex]
		if sel ~= nil then
			view.row_selected = row_key(sel, control.selIndex)
		elseif control.selValue ~= nil then
			for i, value in ipairs(control.list) do
				if value == control.selValue then
					view.row_selected = row_key(value, i)
				end
			end
		end
		-- the other end of a Shift+click range (TimelessJewelListControl.lua:50-58)
		local far = control.highlightIndex and control.list[control.highlightIndex]
		if far ~= nil then
			view.row_highlight = row_key(far, control.highlightIndex)
		end
		view.note = control.defaultText and text_of(control:GetProperty("defaultText")) or nil
		-- PoB lets the user drag rows to reorder this list (left out otherwise, so the view
		-- keeps the shape the item gate compares)
		view.reorder = control.isMutable and true or nil
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
		-- what Enter clicks when no control is selected (PopupDialog.lua:79-83)
		if type(popup.enterControl) == "string" then
			view.enter = popup.enterControl
		end
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
		-- the box's text as the user typed it, key by key
		glue.paste_into(control, value, true)
	elseif (kind == "row" or kind == "row_double" or kind == "row_shift") and control_kind == "List" then
		-- a click on the row (ListControl.lua:338-372): a double click is PoB's second click, and
		-- Shift is held for a range pick (`OverrideSelectIndex`, which then takes the click)
		glue.list_sync(control)
		for i, entry in ipairs(control.list) do
			if row_key(entry, i) == value then
				local real = IsKeyDown
				if kind == "row_shift" then
					IsKeyDown = function(key)
						return key == "SHIFT"
					end
				end
				local ok, err = pcall(function()
					if control:SelectIndex(i) ~= false and control.OnSelClick then
						control:OnSelClick(i, entry, kind == "row_double")
					end
				end)
				IsKeyDown = real
				if not ok then
					error(err, 0)
				end
				return
			end
		end
		error("no such choice", 0)
	elseif kind == "row_move" and control_kind == "List" then
		-- a drag inside a mutable list, and the drop (ListControl.lua:425-437); `label` is the
		-- row's 0-based place after it
		if not control.isMutable then
			error("PoB doesn't offer that here", 0)
		end
		local from
		for i, entry in ipairs(control.list) do
			if row_key(entry, i) == value then
				from = i
			end
		end
		if not from then
			error("no such choice", 0)
		end
		local to = (tonumber(label) or -1) + 1
		if to < 1 or to > #control.list then
			error("no such place", 0)
		end
		if to ~= from then
			local entry = control.list[from]
			control:SelectIndex(from)
			table.remove(control.list, from)
			table.insert(control.list, to, entry)
			if control.OnOrderChange then
				control:OnOrderChange(from, to)
			end
			control.selValue = nil
		end
	else
		error("no such control", 0)
	end
end

-- A control outside a popup as the views show it (`PopupControl`), positions as `GetPos()`
-- gives them; nil for a kind the views don't have
function glue.describe_control(name, control)
	local kind = kind_of(control)
	local view = kind and control_view(name, control, kind, 0, 0)
	if view and view.row_keys then
		for i, key in ipairs(view.row_keys) do
			view.rows[i] = { key = key, label = view.row_labels[i] }
		end
		view.row_keys, view.row_labels = nil, nil
	end
	return view, kind
end

-- While a timeless search runs, only its window's Cancel is offered: PoB's press is
-- synchronous, so nothing else could change what the search reads
local function refuse_while_searching(name)
	if is_timeless(session) and glue.timeless_running() and name ~= "closeButton" then
		error("a timeless search is already running", 0)
	end
end

-- Sets a control as the user would; see `glue.set_control`. Returns whether PoB asked for a
-- recalculation (the Mod Browser's double click adds the mod, R28).
function glue.popup_set(name, kind, value, label)
	require_session()
	refuse_while_searching(name)
	local flag_before = pob.build and pob.build.buildFlag and true or false
	local trading = is_trade(session)
	if is_timeless(session) and name == "realmSelection" then
		timeless_leagues(pob.build.treeTab)
	end
	with_session(function()
		local control, control_kind = find(name)
		require_usable(control)
		if not trading then
			glue.set_control(control, control_kind, kind, value, label)
			return
		end
		no_launches(function()
			glue.set_control(control, control_kind, kind, value, label)
			-- a realm pick refetches the leagues, which leaves "Loading..." (R10)
			if name == "realmDrop" and not is_timeless(session) then
				buy_leagues(session.popup)
			end
		end)
	end)
	return (pob.build and pob.build.buildFlag and not flag_before) and true or false
end

-- Presses a button. Returns whether the session ended and whether PoB asked for a
-- recalculation (`buildFlag`, which a tattoo sets).
function glue.popup_press(name)
	require_session()
	if session.kind == "GameVersion" then
		return game_version_press(name)
	end
	local flag_before = pob.build.buildFlag and true or false
	refuse_while_searching(name)
	local options = session.kind == "Options"
	-- Options' Save writes Settings.xml: it is refused, with the window and its edits left
	-- open, when the write would lose another program's changes or never happen (I-1)
	if options and name == "save" then
		glue.settings_check_writable("save Options")
	end
	local trading = is_trade(session)
	local timeless = is_timeless(session)
	session.pressing = true
	local url
	local started = false
	local ok, ended = pcall(with_session, function()
		local control, kind = find(name)
		if kind ~= "Button" then
			error("no such control", 0)
		end
		require_usable(control)
		if timeless and name == "searchButton" then
			-- the search runs as a background job; the press returns at once (R17)
			glue.timeless_start(session.popup, control)
			started = true
		elseif trading then
			-- the user's own press: a trade link it opens goes to the UI (R20)
			url = no_launches(function()
				return glue.capture_url(control.onClick)
			end)
		else
			control.onClick()
		end
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
	return { ended = ended, build_flag = (pob.build.buildFlag and not flag_before) and true or false, url = url, started = started }
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
	drop()
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
		-- a list row's tooltip, as hovering the row builds it (ListControl.lua:319-321)
		if control and not control.tooltipFunc and control.AddValueTooltip and index and control:IsShown() then
			local value = control.list and control.list[index + 1]
			if value == nil then
				return
			end
			local tooltip = new("Tooltip")
			control:AddValueTooltip(tooltip, index + 1, value)
			out = glue.read_tooltip(tooltip)
			return
		end
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
