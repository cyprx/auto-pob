-- PoB's list builders for the lists with a coroutine builder (`ItemDBControl`, the anoint
-- `NotableDBControl`): the draw-only sync Draw does before a list is read (R14, R17).

glue = glue or {}

-- Draw's preamble: restart the builder on a new flag or output revision. A restart wipes
-- the list, so rows are only read once the builder has ended.
local function restart(control)
	local tab = control.itemsTab
	if tab and tab.build.outputRevision ~= control.listOutputRevision then
		control.listBuildFlag = true
	end
	if control.listBuildFlag then
		control.listBuildFlag = false
		wipeTable(control.list)
		control.listBuilder = coroutine.create(control.ListBuilder)
		control.listOutputRevision = tab and tab.build.outputRevision
	end
end

-- Resumes the builder once, as Draw does; true when it has ended. A failed resume is an
-- error, where PoB swallows it outside dev mode.
local function resume(control)
	local ok, err = coroutine.resume(control.listBuilder, control)
	if not ok then
		control.listBuilder = nil
		error(tostring(err), 0)
	end
	if coroutine.status(control.listBuilder) == "dead" then
		control.listBuilder = nil
		return true
	end
	return false
end

-- Whether the list has builder work: a pending restart or a live builder
function glue.list_pending(control)
	local tab = control.itemsTab
	return (control.listBuildFlag or control.listBuilder ~= nil
		or (tab ~= nil and tab.build.outputRevision ~= control.listOutputRevision)) and true or false
end

-- What Draw does before a list control reads its rows. A name sort never yields, so the
-- builder is run to its end here; a stat sort is a background job.
function glue.list_sync(control)
	if not control.ListBuilder then
		return
	end
	restart(control)
	-- Draw doesn't resume a list while its database loads
	if control.db and control.db.loading then
		return
	end
	if control.listBuilder and not (control.sortDetail and control.sortDetail.stat) then
		resume(control)
	end
end

-- One resume of the list builder after Draw's restart check. `percent` comes from PoB's
-- own "Sorting... (N%)" text.
function glue.list_step(control)
	restart(control)
	if control.db and control.db.loading then
		return { finished = false }
	end
	if not control.listBuilder then
		return { finished = true }
	end
	local finished = resume(control)
	local percent
	if not finished then
		percent = tonumber(tostring(control.defaultText or ""):match("%((%d+)%%%)"))
	end
	return { finished = finished, percent = percent }
end

-- ---- The item databases (R13-R16) ----

local DATABASES = { Uniques = "uniqueDB", Rares = "rareDB" }

local function items_tab()
	return pob.build.itemsTab
end

local function db_control(db)
	local name = DATABASES[db]
	if not name then
		error("no such database", 0)
	end
	return items_tab().controls[name]
end

-- Queries opened while the database loads: the leagues and types the filters index aren't
-- known yet, so they are applied once it has loaded
local waiting = {}

local check_query, apply_query

-- What Draw does once the database has loaded: the leagues and types the filters list. A
-- query opened while it loaded is applied now; returns true then.
local function prepare(db, control)
	if control.db.loading then
		return false
	end
	if not control.leaguesAndTypesLoaded then
		control:LoadLeaguesAndTypes()
	end
	local query = waiting[db]
	if query then
		waiting[db] = nil
		check_query(control, query)
		apply_query(control, query)
		return true
	end
	return false
end

local function check_choice(dropdown, index)
	if index < 0 or index >= #dropdown.list then
		error("no such choice", 0)
	end
end

-- Refuses a query PoB's controls can't take; nothing is set. The leagues and types are only
-- checked once they are known.
function check_query(control, q)
	local c = control.controls
	if control.dbType == "RARE" and (q.league ~= 0 or q.requirement ~= 0 or q.obtainable ~= 0 or q.sort ~= 0) then
		error("the rares database has no such filter", 0)
	end
	check_choice(c.slot, q.slot)
	check_choice(c.searchMode, q.search_mode)
	if control.leaguesAndTypesLoaded then
		check_choice(c.type, q.kind)
		if c.league then
			check_choice(c.league, q.league)
		end
	end
	if c.sort then
		check_choice(c.requirement, q.requirement)
		check_choice(c.obtainable, q.obtainable)
		check_choice(c.sort, q.sort)
	end
	if #q.search > c.search.limit then
		error("the text is too long", 0)
	end
end

-- Sets PoB's controls as a user would: a dropdown only when it changes (its selFunc flags the
-- list), the search box by typing
function apply_query(control, q)
	local c = control.controls
	local function choose(dropdown, index)
		if dropdown.selIndex ~= index + 1 then
			dropdown:SetSel(index + 1)
		end
	end
	if c.search.buf ~= q.search then
		glue.type_into(c.search, q.search)
	end
	choose(c.slot, q.slot)
	choose(c.type, q.kind)
	if c.sort then
		choose(c.league, q.league)
		choose(c.requirement, q.requirement)
		choose(c.obtainable, q.obtainable)
		choose(c.sort, q.sort)
	end
	choose(c.searchMode, q.search_mode)
end

-- Opens or replaces the search. `restart` rebuilds the list although no control changed (the
-- last sort failed). Nothing is changed when the query is refused.
function glue.item_db_open(db, q, restart)
	local control = db_control(db)
	prepare(db, control)
	check_query(control, q)
	if control.db.loading then
		waiting[db] = q
	else
		waiting[db] = nil
		apply_query(control, q)
	end
	if restart then
		control.listBuildFlag = true
	end
end

function glue.item_db_close()
	waiting = {}
end

-- Whether the open database has work: its load, or a stat sort to run
function glue.item_db_pending(db)
	local control = db_control(db)
	if control.db.loading then
		return true
	end
	if waiting[db] then
		return true
	end
	return control.sortDetail ~= nil and control.sortDetail.stat ~= nil and glue.list_pending(control)
end

-- One slice of the open database's work: PoB's own load, then its list builder. `finished`
-- is true when the load ended or the list is complete, which is when the panel has news.
function glue.item_db_step(db)
	local control = db_control(db)
	if control.db.loading then
		local load = pob.main.main.onFrameFuncs.LoadItems
		if not load then
			error("the item database failed to load", 0)
		end
		load()
		if control.db.loading then
			return { finished = false }
		end
	end
	if prepare(db, control) then
		return { finished = true }
	end
	return glue.list_step(control)
end

local function plain(text)
	return (tostring(text or ""):gsub("%^x%x%x%x%x%x%x", ""):gsub("%^%d", ""))
end

-- The panel as PoB's controls hold it. The rows are left out until the list builder has
-- ended: Draw wipes the list when it restarts.
function glue.item_db_view(db)
	local control = db_control(db)
	local c = control.controls
	local loading = control.db.loading and true or false
	local sorting = false
	if loading then
		control.defaultText = "^7Loading..."
	else
		prepare(db, control)
		glue.list_sync(control)
		sorting = glue.list_pending(control)
	end
	local stat = control.sortDetail ~= nil and control.sortDetail.stat ~= nil
	local rows = {}
	local percent
	if sorting then
		percent = tonumber(tostring(control.defaultText or ""):match("%((%d+)%%%)"))
	elseif not loading then
		for i, item in ipairs(control.list) do
			local power = stat and item.measuredPower
			if power and (power == math.huge or power == -math.huge or power ~= power) then
				power = nil
			end
			rows[i] = {
				name = item.name,
				label = control:GetRowValue(1, i, item),
				base = item.baseName or "",
				sort_value = power or nil,
			}
		end
	end
	return {
		db = db,
		loading = loading,
		sorting = sorting,
		percent = percent,
		slot = glue.drop_list(c.slot),
		kind = glue.drop_list(c.type),
		league = c.league and glue.drop_list(c.league) or nil,
		requirement = c.requirement and glue.drop_list(c.requirement) or nil,
		obtainable = c.obtainable and glue.drop_list(c.obtainable) or nil,
		sort = c.sort and glue.drop_list(c.sort) or nil,
		search = c.search.buf,
		search_mode = glue.drop_list(c.searchMode),
		rows = rows,
		note = plain(control.defaultText),
	}
end

-- The database item with this name. It is found in the database, not in the shown list, which
-- is empty while a sort runs (R15).
local function find_item(db, name)
	local control = db_control(db)
	if control.db.loading then
		error("the item database is still loading", 0)
	end
	local item = control.db.list[name]
	if not item then
		error("no such item in the database", 0)
	end
	local index = 1
	for i, listed in ipairs(control.list) do
		if listed == item then
			index = i
		end
	end
	return control, item, index
end

-- The draw-only updates the tab's Draw makes before a click acts on its slots (SP5a R16)
local function sync_tab()
	items_tab().controls.itemList:UpdateList()
	items_tab():UpdateSockets()
end

-- The double click: the item goes to the display panel
function glue.item_db_view_item(db, name)
	local control, item, index = find_item(db, name)
	sync_tab()
	control:OnSelClick(index, item, true)
end

-- Ctrl+click: PoB adds the item and equips it. The key state is swapped for the call and
-- always put back (R15).
function glue.item_db_add(db, name)
	local control, item, index = find_item(db, name)
	sync_tab()
	local saved = IsKeyDown
	IsKeyDown = function(key)
		return key == "CTRL"
	end
	local ok, err = pcall(control.OnSelClick, control, index, item, false)
	IsKeyDown = saved
	if not ok then
		error(err, 0)
	end
end

-- What hovering a row shows; nil for a vanished item or a database still loading
function glue.item_db_tooltip(db, name)
	local control = db_control(db)
	local item = not control.db.loading and control.db.list[name]
	if not item then
		return nil
	end
	local tooltip = new("Tooltip")
	control:AddValueTooltip(tooltip, 1, item)
	return glue.read_tooltip(tooltip)
end
