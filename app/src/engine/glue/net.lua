-- Network requests (R4, R10-R14).
--
-- A request is what one press of the user's started: every subscript launched during
-- `glue.net_begin`'s function, and every one launched inside their callbacks (a token refresh
-- before a download, the profile page after a character list). It ends when its last
-- subscript has been delivered.
--
-- Launches need an open scope (`__net_scope`, checked by the native in net.rs), which only
-- `net_begin` and `net_dispatch` open, and no `Build:Init` running (`__net_in_init`). Every
-- callback runs with PoB's popups and `promptMsg` guarded: what it leaves behind becomes the
-- request's error, and `#main.popups == 0` afterwards.

glue = glue or {}

local native_launch = LaunchSubScript

-- request id -> { id, label, opts, scope, popup, subs, cancelled, new_build, error, last_error }
local requests = {}
-- pending request ids, in the order they began
local pending = {}
-- subscript id -> request
local by_sub = {}
-- finished entries not yet sent
local finished = {}
-- the request whose press or callback is running (`guarded`)
local current
local next_id = 1
-- Build:Init runs so far (the wrapper below counts them)
local inits = 0
-- what dispatches did to the build since the last `net_take_effects`
local effects = { rebuilt = false, edited = false }

local function main()
	return pob.main.main
end

-- ---- Build:Init (R4, R14) ----

-- Init builds a new ImportTab, whose constructor validates (and maybe refreshes) a stored
-- token. No launch may happen while it runs, even inside a request; and a request whose
-- callback ran it made a new build.
--
-- An Init that couldn't open its build is taken care of here, for every caller (an open, an
-- import's code or link, the Game Version window's Convert):
-- - A build PoB can't load makes Init close it (`CloseBuild`, Build.lua:953-956), often with a
--   popup that says why (TreeTab.lua:507-509's unknown tree version). The popup is closed and
--   its text kept as the reason (`glue.take_init_failure`), the switch to the build list is
--   cancelled and a new build takes the failed one's place. The half-loaded tabs are dropped
--   first: TreeTab:Load empties the spec list before it fails, and the next Init would read it
--   (Build.lua:853-854).
-- - A build made for another game version stops before its tabs are made and opens PoB's
--   "Game Version" window (Build.lua:102-105, 1323-1350). The window becomes a session, and a
--   new build stands behind it, not the previous build's tabs under the new name.
local init_failure

-- Returns true when the Init that just ran left no build of its own, plus the Game Version
-- window and its Convert's arguments when that is why
local function settle_init(self, depth, newMode_before)
	local m = main()
	local popups = m.popups
	local conversion, reason
	for i = #popups - depth, 1, -1 do
		local popup = popups[i]
		if popup.title == "Game Version" and popup.controls.convert then
			conversion = popup
		elseif not reason then
			reason = glue.popup_text(popup)
		end
	end
	local closed = m.newMode == "LIST" and newMode_before ~= "LIST"
	local converting = conversion ~= nil and self.targetVersion == nil
	if not closed and not converting then
		return false
	end
	while #popups > depth do
		m:ClosePopup()
	end
	self.treeTab = nil
	if closed then
		m.newMode = nil
		init_failure = reason or "couldn't open that build"
		return true
	end
	-- PoB's Convert reopens the same file converted, or an import without its text
	-- (Build.lua:1340-1343). No nil in the list: `SetMode` unpacks it with `#`.
	return true, conversion, { self.dbFileName or false, self.buildName, false, true }
end

-- PoB opens the last build Settings.xml names at start (Main.lua:510-523), in bootstrap's first
-- frame, before the wrapper below exists. What that open left is settled the same way once the
-- glue is loaded: a build that failed (PoB went to its list) loses its half-loaded tabs, and
-- one that needs PoB's Game Version window gets it as a session, with a new build behind it.
-- No popup PoB opened at start stays in its stack: there is no window to show it in.
function glue.settle_startup()
	local m = main()
	local popups = m.popups
	local conversion
	for i = #popups, 1, -1 do
		if popups[i].title == "Game Version" and popups[i].controls.convert then
			conversion = popups[i]
		end
	end
	while #popups > 0 do
		m:ClosePopup()
	end
	local build = pob.build
	-- An older auto-pob's GetUserPath ended in a slash, so the build its exit save recorded is
	-- `<user>//Path of Building/...`; PoB cuts the folder out by the length of today's
	-- `buildPath` (Build.lua:65), which would leave it one character off ("/sub/")
	local recorded = m.mode == "BUILD" and build.dbFileName
	if recorded and recorded:find("//", 1, true) then
		local folded = recorded:gsub("/+", "/")
		if folded:sub(1, #m.buildPath) == m.buildPath and build.buildName then
			build.dbFileName = folded
			build.dbFileSubPath = folded:sub(#m.buildPath + 1, -#build.buildName - 5)
		end
	end
	if m.mode == "BUILD" and conversion and build.targetVersion == nil then
		local convert = { build.dbFileName or false, build.buildName, false, true }
		build.treeTab = nil
		build:Init(false, "Unnamed build")
		glue.frame()
		glue.popup_adopt_game_version(conversion, convert)
	elseif m.mode ~= "BUILD" then
		-- a start-up open that failed, or raised, left half-loaded tabs; PoB's tab
		-- constructors run SyncLoadouts against them on the next Init (Build.lua:601-604)
		build.treeTab, build.skillsTab, build.itemsTab, build.configTab = nil, nil, nil, nil
	end
end

-- Why the last Init couldn't open its build; nil when it did (taken once)
function glue.take_init_failure()
	local failure = init_failure
	init_failure = nil
	return failure
end

do
	local build = pob.build
	local init = build.Init
	build.Init = function(self, ...)
		local m = main()
		local depth, newMode = #m.popups, m.newMode
		init_failure = nil
		__net_in_init(true)
		local ret = { pcall(init, self, ...) }
		if not ret[1] then
			-- PoB makes the new tabs one after another, and each constructor's set change
			-- runs SyncLoadouts against the tabs the build holds then (Build.lua:601-604,
			-- 753-859). After a load that raised, those are the half-loaded ones (no active
			-- skill set), and every later Init, of any build, would raise there too. Without
			-- them SyncLoadouts skips its loadout list until the new tabs are in place. A new
			-- build stands in, whichever path loaded this one (an open, a code, a link's
			-- answer); the load's own error is still raised below.
			self.treeTab, self.skillsTab, self.itemsTab, self.configTab = nil, nil, nil, nil
			if not pcall(init, self, false, "Unnamed build") then
				self.treeTab, self.skillsTab, self.itemsTab, self.configTab = nil, nil, nil, nil
			end
		end
		local failed, conversion, convert
		if ret[1] then
			failed, conversion, convert = settle_init(self, depth, newMode)
		end
		if failed then
			ret = { pcall(init, self, false, "Unnamed build") }
		end
		__net_in_init(false)
		inits = inits + 1
		if conversion then
			glue.popup_adopt_game_version(conversion, convert)
		end
		if not ret[1] then
			error(ret[2], 0)
		end
		return unpack(ret, 2, table.maxn(ret))
	end
end

-- How many times PoB has initialised a build: `pob.build` is one object that every `SetMode`
-- initialises again, so this, not its identity, tells one build from the next (R19)
function glue.build_generation()
	return inits
end

local SIGNING_IN = "Signing in"

-- ---- Errors ----

local function first_line(message)
	message = tostring(message)
	return (message:match("^[^\n]*") or message)
end

-- The first error a request meets in its popups, `promptMsg` or our own machinery sticks.
local function record_error(req, message)
	message = first_line(StripEscapes(tostring(message))):gsub("^%s+", ""):gsub("%s+$", "")
	-- an error shown to the user carries no URL or name either (R21)
	if __net_redact then
		message = __net_redact(message)
	end
	-- a site's error page can be long: keep the start, on a character boundary
	if #message > 200 then
		local cut = 200
		while cut > 0 and message:byte(cut + 1) and message:byte(cut + 1) >= 0x80 and message:byte(cut + 1) < 0xC0 do
			cut = cut - 1
		end
		message = message:sub(1, cut) .. "..."
	end
	if not req.error and message ~= "" then
		req.error = message
	end
end

-- ---- Edits (R14) ----

local MOD_FLAGS = { "notesTab", "partyTab", "configTab", "treeTab", "spec", "skillsTab", "itemsTab", "calcsTab" }

local function edit_marks()
	if not glue.build_loaded() then
		return nil
	end
	local build = pob.build
	local marks = { build = build, flag = build.buildFlag and true or false, mod = build.modFlag and true or false }
	for _, name in ipairs(MOD_FLAGS) do
		marks[name] = build[name] and build[name].modFlag and true or false
	end
	return marks
end

local function edited_since(marks)
	if not marks or marks.build ~= pob.build or not glue.build_loaded() then
		return false
	end
	local build = pob.build
	if build.buildFlag and not marks.flag then
		return true
	end
	if (build.modFlag and true or false) ~= marks.mod then
		return true
	end
	for _, name in ipairs(MOD_FLAGS) do
		if (build[name] and build[name].modFlag and true or false) ~= marks[name] then
			return true
		end
	end
	return false
end

-- ---- Popups (R11) ----

local function label_of(control)
	local label = control.label
	if type(label) == "function" then
		local ok, text = pcall(label)
		label = ok and text or nil
	end
	return type(label) == "string" and StripEscapes(label) or nil
end

-- A message popup's text: its labels, in order
local function message_text(popup)
	local lines = {}
	for _, control in ipairs(popup.controls or {}) do
		local text = label_of(control)
		if text and text:match("%S") then
			lines[#lines + 1] = text:gsub("^%s+", ""):gsub("%s+$", "")
		end
	end
	return table.concat(lines, " ")
end

-- Handles the popups a callback left above `depth`, newest first: an owned popup is taken out
-- again and kept, a confirm is pressed for a request begun `confirmed` and cancelled otherwise,
-- and a message becomes the request's error. Returns whether the owned popup is still open.
local function settle_popups(req, depth)
	local popups = main().popups
	local owned_open = false
	local rounds = 0
	while #popups > depth and rounds < 100 do
		rounds = rounds + 1
		local top = popups[1]
		if top == req.popup then
			owned_open = true
			table.remove(popups, 1)
		elseif top.controls and top.controls.confirm and req.opts.confirmed and not top.netConfirmed then
			top.netConfirmed = true
			local ok, err = pcall(top.controls.confirm.onClick)
			if not ok then
				record_error(req, err)
			end
			if popups[1] == top then
				table.remove(popups, 1)
			end
		else
			if not (top.controls and top.controls.confirm) then
				record_error(req, message_text(top))
			end
			table.remove(popups, 1)
		end
	end
	while #popups > depth do
		table.remove(popups, 1)
	end
	return owned_open
end

-- ---- Chains ----

local function attach(req, id)
	req.subs[id] = true
	by_sub[id] = req
end

-- PoB refreshes the token again after every 401 that follows a good refresh (PoEAPI.lua:54
-- resets `retries`, :164 retries while it is 0), so a 401 the refresh can't cure would loop
-- forever. Once a request has made this many refreshes, each download it launches next is
-- delivered with `api.retries = 1`: a 401 then takes PoB's own last branch (:169-182), which
-- finishes the rate limiter's request and gives PoB's own error. A deviation from PoB, to
-- spare GGG's API.
local REFRESH_LIMIT = 2
local TOKEN_URL = "https://www.pathofexile.com/oauth/token"

-- Runs `fn` with launches recorded into `req`, the native gate open when `open`.
local function in_scope(req, open, fn)
	local saved = LaunchSubScript
	LaunchSubScript = function(script, blocking, nonblocking, url, header, body, ...)
		local refresh = url == TOKEN_URL and type(body) == "string"
			and body:match("^client_id=pob&grant_type=refresh_token") ~= nil
		local id = native_launch(script, blocking, nonblocking, url, header, body, ...)
		if id then
			attach(req, id)
			if refresh then
				req.refreshes = (req.refreshes or 0) + 1
			elseif (req.refreshes or 0) >= REFRESH_LIMIT then
				req.last_leg = req.last_leg or {}
				req.last_leg[id] = true
			end
		end
		return id
	end
	if open then
		__net_scope(1)
	end
	local ok, err = pcall(fn)
	if open then
		__net_scope(-1)
	end
	LaunchSubScript = saved
	return ok, err
end

-- Runs `fn(...)` for `req` with its owned popup at `main.popups[1]`, launches recorded into
-- it, and popups, `promptMsg` and build changes taken stock of afterwards. Returns true plus
-- `fn`'s results, or false plus its error (also recorded on the request).
local function guarded(req, open, fn, ...)
	local popups = main().popups
	local depth = #popups
	local args = { n = select("#", ...), ... }
	local inits_before = inits
	-- taken before the owned popup goes on the stack, and not allowed to raise: a failure here
	-- must leave neither the popup nor the request's bookkeeping behind
	local marked, marks = pcall(edit_marks)
	if not marked then
		marks = nil
	end
	if req.popup then
		table.insert(popups, 1, req.popup)
	end
	local outer = current
	current = req
	-- PoB's callbacks close "their" popup with main:ClosePopup(), which takes the top one: only
	-- what this request put on the stack may go, never a popup below it (a session window the
	-- user opened while the request ran is kept off the stack, and stays so)
	local mainObject = main()
	-- the instance's own field, nil when the class method serves: put back as it was
	local ownClose = rawget(mainObject, "ClosePopup")
	local closePopup = mainObject.ClosePopup
	mainObject.ClosePopup = function(self, ...)
		if #self.popups > depth then
			return closePopup(self, ...)
		end
	end
	local results
	local ok, err = in_scope(req, open, function()
		results = { pcall(fn, unpack(args, 1, args.n)) }
		if not results[1] then
			record_error(req, results[2])
		end
		-- a confirm pressed here may launch the next step of the chain
		local owned_open = settle_popups(req, depth)
		if req.popup and not owned_open then
			req.popup = nil
		end
	end)
	current = outer
	rawset(mainObject, "ClosePopup", ownClose)
	if not ok then
		record_error(req, err)
	end
	-- whatever happened, the stack is as it was
	settle_popups(req, depth)
	-- Launch.lua shows a PCall'd callback's error as a prompt
	local prompt = pob.main.promptMsg
	if prompt then
		pob.main.promptMsg = nil
		record_error(req, glue.error_reason(prompt))
	end
	if inits ~= inits_before then
		-- the callback's Init couldn't open the build: a new one stands in, and PoB's reason
		-- is the request's
		local failure = glue.take_init_failure()
		if failure then
			record_error(req, failure)
			req.init_failure = failure
		end
		effects.rebuilt = true
		effects.new_build_req = req.id
		req.new_build = true
	elseif edited_since(marks) then
		effects.edited = true
	end
	if results and results[1] then
		return true, unpack(results, 2, table.maxn(results))
	end
	return false, results and results[2] or err
end

local function remove_pending(id)
	for i, pending_id in ipairs(pending) do
		if pending_id == id then
			table.remove(pending, i)
			return
		end
	end
end

local function finish(req)
	-- an owned popup still open at the end shows why in its `msg` label (R11), unless the
	-- subscript itself failed: PoB drops a CUSTOM subscript's error without calling back, so
	-- the label still shows what it showed when the press began ("Resolving PoEURL link...")
	if req.popup then
		if req.opts.done and not req.cancelled then
			local ok, err = pcall(req.opts.done, req)
			if not ok then
				record_error(req, err)
			end
		end
		local msg = req.popup.controls and req.popup.controls.msg
		local text = msg and label_of(msg)
		if text and text:match("%S") and not req.last_error then
			record_error(req, text)
		end
		req.popup = nil
	end
	-- what PoB's own flags say about a request that ended without its effect (R16)
	if req.opts.after and not req.cancelled then
		local ok, err = pcall(req.opts.after, req)
		if not ok then
			record_error(req, err)
		end
	end
	requests[req.id] = nil
	remove_pending(req.id)
	local entry = {
		id = req.id,
		label = req.label,
		cancelled = req.cancelled,
		new_build = req.new_build,
	}
	if req.cancelled then
		entry.error = "Cancelled"
	else
		entry.error = req.error or req.last_error
	end
	finished[#finished + 1] = entry
end

local function detach(req, id)
	by_sub[id] = nil
	req.subs[id] = nil
	if requests[req.id] and next(req.subs) == nil then
		finish(req)
	end
end

-- ---- The API ----

-- Begins a request labelled `label` (R10) and runs `fn`, the user's press, inside it.
-- `opts.confirmed`: confirms PoB's confirm popups; `opts.popup`: a detached PopupDialog the
-- request owns (taken out of `main.popups` if it is on top); `opts.scope`: "build" (default) or
-- "list"; `opts.done(req)`: called when the request ends with its owned popup still open (not
-- when cancelled), before `req.error` is read from the popup; `opts.after(req)`: called when the
-- request ends, not cancelled, once its error is known (it may set `req.error`). Returns the request id, or nil plus PoB's message when nothing was launched.
-- An error `fn` raises before launching anything is raised.
function glue.net_begin(label, opts, fn)
	opts = opts or {}
	local popups = main().popups
	if opts.popup and popups[1] == opts.popup then
		table.remove(popups, 1)
	end
	local req = {
		id = next_id,
		label = label,
		opts = opts,
		scope = opts.scope or "build",
		popup = opts.popup,
		subs = {},
		cancelled = false,
		new_build = false,
	}
	next_id = next_id + 1
	requests[req.id] = req
	-- the command that begins a request answers for what its press did to the build
	local saved_effects = effects
	effects = { rebuilt = false, edited = false }
	local ok, err = guarded(req, true, fn)
	effects = saved_effects
	if next(req.subs) == nil then
		requests[req.id] = nil
		req.popup = nil
		if not ok then
			error(first_line(err), 0)
		end
		-- the third: why an Init the press ran couldn't open its build
		return nil, req.error, req.init_failure
	end
	pending[#pending + 1] = req.id
	return req.id
end

-- Delivers a subscript event through PoB's `launch:OnSub*` (R11): `kind` is "finished"
-- (with the return values), "error" (with the message) or "call" (with the function name and
-- its arguments; returns the call's results).
function glue.net_dispatch(kind, id, ...)
	local req = by_sub[id]
	local launch = pob.main
	if not req then
		-- not ours (a request finished or cancelled under it): PoB forgets it
		if kind ~= "call" then
			launch.subScripts[id] = nil
		end
		return
	end
	if kind == "call" then
		-- A subscript's ConPrintf is dropped, not logged: DownloadPage logs the page it
		-- fetches, and a URL can carry an account name (R21).
		if (...) == "ConPrintf" then
			return
		end
		local results = { guarded(req, true, function(...)
			return runCallback("OnSubCall", ...)
		end, ...) }
		if results[1] then
			return unpack(results, 2, table.maxn(results))
		end
		return
	end
	local entry = launch.subScripts[id]
	-- the refresh limit: PoB's next 401 ends the chain itself (see REFRESH_LIMIT); only a
	-- subscript PoB still knows is delivered, so only then does PoB's state change
	local function limit_refresh()
		if req.last_leg and req.last_leg[id] and main().api then
			main().api.retries = 1
		end
	end
	if kind == "finished" then
		-- a download hands its callback (body, errMsg, header)
		if entry and entry.type == "DOWNLOAD" then
			local _, message = ...
			req.last_error = message and __net_redact(first_line(message)) or nil
		else
			req.last_error = nil
		end
		if entry then
			limit_refresh()
			guarded(req, true, function(...)
				runCallback("OnSubFinished", id, ...)
			end, ...)
		end
	elseif kind == "error" then
		local message = first_line(...)
		-- PoB's listener (LaunchServer.lua:162) concatenates the redirect's `error` and
		-- `error_description`, and raises on a redirect that carries neither them nor a code.
		-- The sign-in ends as PoB ends one with no code (PoEAPI.lua:112): "No auth token"
		-- (nil here makes PoB's callback say it).
		local reply = message
		if req.label == SIGNING_IN and entry and entry.type == "DOWNLOAD" and message:find("attempt to concatenate", 1, true) then
			reply = nil
			message = "No auth token"
		end
		req.last_error = __net_redact and __net_redact(message) or message
		-- PoB drops a CUSTOM subscript's error without calling back; the request ends anyway
		if entry then
			limit_refresh()
			guarded(req, true, function()
				runCallback("OnSubError", id, reply)
			end)
		end
	end
	launch.subScripts[id] = nil
	detach(req, id)
end

-- Ends subscript `id` (already aborted, or whose dispatch failed) through PoB's own failure
-- path with `message` (R12): a DOWNLOAD through OnSubError, a CUSTOM through
-- OnSubFinished(id, nil, message), since OnSubError drops a CUSTOM without calling back.
-- A cancelled request's callbacks run with the scope closed, so nothing new launches.
function glue.net_fail(id, message)
	local req = by_sub[id]
	local launch = pob.main
	local entry = launch.subScripts[id]
	if entry then
		local target = req or { id = 0, opts = {}, subs = {} }
		guarded(target, req ~= nil and not req.cancelled, function()
			if entry.type == "CUSTOM" then
				runCallback("OnSubFinished", id, nil, message)
			else
				runCallback("OnSubError", id, message)
			end
		end)
	end
	launch.subScripts[id] = nil
	if req then
		record_error(req, message)
		detach(req, id)
	end
end

-- How many requests were cancelled so far
local cancels = 0

function glue.net_epoch()
	return cancels
end

-- The running subscripts of request `id`, which is marked cancelled; the caller aborts them
-- and ends each with `net_fail(sub, "Cancelled")`.
function glue.net_live_ids(id)
	local req = requests[id]
	if not req then
		-- it ended while the cancel was on its way: nothing is left to cancel
		return {}
	end
	req.cancelled = true
	cancels = cancels + 1
	local ids = {}
	for sub in pairs(req.subs) do
		ids[#ids + 1] = sub
	end
	table.sort(ids)
	return ids
end

local function cancel_ids(ids)
	for _, id in ipairs(ids) do
		if requests[id] then
			for _, sub in ipairs(glue.net_live_ids(id)) do
				AbortSubScript(sub)
				glue.net_fail(sub, "Cancelled")
			end
		end
	end
end

-- Cancels every pending request, or only those of `scope` (R13).
function glue.net_cancel_all(scope)
	local ids = {}
	for _, id in ipairs(pending) do
		if not scope or requests[id].scope == scope then
			ids[#ids + 1] = id
		end
	end
	cancel_ids(ids)
end

-- A callback made a new build (R13): every other pending request is cancelled, as a command
-- that replaces the build does, so no late result lands on the new build. The request that
-- made the build goes on.
function glue.net_cancel_replaced()
	if not effects.rebuilt then
		return
	end
	local ids = {}
	for _, id in ipairs(pending) do
		if id ~= effects.new_build_req then
			ids[#ids + 1] = id
		end
	end
	cancel_ids(ids)
end

function glue.net_pending()
	return #pending
end

-- The pending requests, and the finished ones not sent yet (each is sent once).
function glue.requests_view()
	local view = {}
	if #pending > 0 then
		view.pending = {}
		for _, id in ipairs(pending) do
			view.pending[#view.pending + 1] = { id = id, label = requests[id].label }
		end
	end
	if #finished > 0 then
		view.finished = finished
		finished = {}
	end
	return view
end

-- ---- The sign-in link (R1) ----

local AUTH_URL = "^https://www%.pathofexile%.com/oauth/authorize%?"

-- `OpenURL`, as bootstrap.lua hands it over: the engine never opens a browser. PoB's sign-in
-- link is kept for the UI, and only while a "Signing in" request runs the call; any other URL
-- is dropped.
function glue.on_open_url(url)
	local req = current
	if req and requests[req.id] and req.label == SIGNING_IN and type(url) == "string" and url:match(AUTH_URL) then
		req.auth_url = url
	end
end

-- The first pending request labelled `label`, if any: { id, auth_url }
function glue.net_find(label)
	for _, id in ipairs(pending) do
		local req = requests[id]
		if req.label == label then
			return { id = id, auth_url = req.auth_url }
		end
	end
end

-- The pending sign-in, if any: { id, auth_url }
function glue.net_sign_in()
	return glue.net_find(SIGNING_IN)
end

-- What the dispatches since the last call did to the build (R14).
function glue.net_take_effects()
	local taken = effects
	effects = { rebuilt = false, edited = false }
	return taken
end
