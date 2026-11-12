-- Build sites: PoB's import box, Share and share codes (R15-R17). Runs after net.lua.
--
-- With a build open everything goes through the build's own ImportTab controls (the box's
-- validator, its Go button, the Share button), pressed inside `glue.net_begin`. With none (the
-- build list) there is no ImportTab, so the link is fetched with `buildSites.DownloadBuild`
-- and opened with `SetMode`, as `Main:Init` does for a protocol link.

glue = glue or {}

local function main()
	return pob.main.main
end

-- The ImportTab of the open build
local function import_tab()
	if not glue.build_loaded() then
		error("open a build first", 0)
	end
	return pob.build.importTab
end

local function trim(text)
	return (tostring(text):gsub("^%s+", ""):gsub("%s+$", ""))
end

local INVALID = "that isn't a link PoB can import"

-- PoB's verdict on the box, without colour codes; "Invalid input" says nothing could be imported
local function import_detail(tab)
	return StripEscapes(tab.importCodeDetail or "")
end

-- Why nothing was imported, from the box's verdict (R16)
local function not_imported(tab)
	local detail = import_detail(tab)
	if detail == "" or detail == "Invalid input" then
		return INVALID
	end
	return detail
end

-- ---- Views ----

-- The share link the last Share made, kept while its text is still in the code box: { tab, link }
local shared

function glue.share_view()
	local tab = import_tab()
	local controls = tab.controls
	local sites = {}
	for _, site in ipairs(controls.exportFrom.list) do
		sites[#sites + 1] = { id = site.id, label = site.label }
	end
	local last_site = tab.exportWebsiteSelected or pob.main.main.lastExportWebsite
	local known = false
	for _, site in ipairs(sites) do
		known = known or site.id == last_site
	end
	local link
	if shared and shared.tab == tab and controls.generateCodeOut.buf == shared.link then
		link = shared.link
	end
	local site
	if tab.importCodeSite then
		local entry = buildSites.websiteList[tab.importCodeSite]
		site = entry and entry.id
	end
	local state = controls.importCodeState.label
	return {
		sites = sites,
		last_site = known and last_site or nil,
		link = link,
		sharing = controls.generateCodeByLink.label == "Creating link...",
		import = {
			text = controls.importCodeIn.buf,
			valid = tab.importCodeValid and true or false,
			detail = StripEscapes(type(state) == "function" and state() or state or ""),
			site = site,
			fetching = tab.importCodeFetching and true or false,
			can_import_to_this_build = pob.build.dbFileName and true or false,
		},
	}
end

-- PoB's Generate button (ImportTab.lua:511); the UI copies the code, as PoB's Copy button does
-- (which clears the box).
function glue.share_code()
	local controls = import_tab().controls
	controls.generateCode.onClick()
	local code = controls.generateCodeOut.buf
	controls.generateCodeOut:SetText("")
	return code
end

-- Sets a check box as a click would; the boxes may have no callback (PoB's
-- CheckBoxControl:Click skips a missing one too)
local function option(box, on)
	if box.state ~= on then
		box.state = on
		if box.changeFunc then
			box.changeFunc(on)
		end
	end
end

-- ---- The import box ----

function glue.set_import_code(text)
	import_tab().controls.importCodeIn:SetText(tostring(text), true)
end

-- The raw code a site served, as an XML text (ImportTab.lua:605), or nil
local function decode(body)
	local ok, xml = pcall(function()
		return Inflate(common.base64.decode((body:gsub("-", "+"):gsub("_", "/"))))
	end)
	if ok and xml and xml ~= "" then
		return xml
	end
end

-- A link from the build list: no ImportTab exists, so the download and the new build are
-- done as `Main:Init` does for `arg[1]` (R15). The request belongs to the list.
local function import_into_new_build(link, site)
	local id, message = glue.net_begin("Fetching build from " .. site.label, { scope = "list" }, function()
		buildSites.DownloadBuild(link, site, function(ok, data, importLink)
			-- a failed download is the request's error (curl's or "Response code: n")
			if not ok then
				return
			end
			local xml = decode(data)
			if not xml then
				main():OpenMessagePopup("Import", INVALID)
				return
			end
			-- Main.lua:70-77 stores the callback's third argument (the URL it fetched), which
			-- shadows the parsed protocol link
			glue.enter_build(false, "Imported Build", xml, false, importLink)
		end)
	end)
	if not id then
		error(message or "couldn't start the request", 0)
	end
	return id
end

-- What `glue.import_check` just put through PoB's validator, so the press that follows
-- doesn't run it a second time on the same text: { tab, url, buf, epoch }
local checked

-- Puts `url` in the box as pasting it does (PoB's validator runs), unless `glue.import_check`
-- just did with the same text and nothing was cancelled since (a cancel runs PoB's failure
-- paths, which may touch the box's verdict).
local function put_text(tab, url)
	local box = tab.controls.importCodeIn
	local last = checked
	checked = nil
	if last and last.tab == tab and last.url == url and last.buf == box.buf
		and last.epoch == glue.net_epoch() and tab.importCodeValid then
		return
	end
	box:SetText(url, true)
end

-- With a build open: the box takes the text, the mode drop-down the target, Go does the rest.
local function import_through_tab(url, mode)
	local tab = import_tab()
	local controls = tab.controls
	if mode == "this" and not pob.build.dbFileName then
		error("this build has no file yet; import into a new build", 0)
	end
	put_text(tab, url)
	if not tab.importCodeValid then
		error(not_imported(tab), 0)
	end
	-- PoB enables the drop-down only for a build with a file, or once "Import as comparison" is
	-- picked; its validator puts a build with no file back on "new" (ImportTab.lua:585, :681)
	if mode == "this" or mode == "compare" then
		if not controls.importCodeMode:IsEnabled() then
			error("PoB doesn't offer that here", 0)
		end
		controls.importCodeMode:SetSel(mode == "this" and 1 or 3)
	else
		controls.importCodeMode:SetSel(2)
	end
	if not controls.importCodeGo:IsShown() or not controls.importCodeGo:IsEnabled() then
		error("PoB doesn't offer that here", 0)
	end
	local label = "Importing build"
	local site = tab.importCodeSite and not tab.importCodeXML and buildSites.websiteList[tab.importCodeSite]
	if site then
		label = "Fetching build from " .. site.label
	end
	-- a comparison adds an entry to the Compare tab and leaves the build as it is (R5)
	local compare = mode == "compare" and pob.build.compareTab
	local entries = compare and #compare.compareEntries
	local function compared()
		return compare and #compare.compareEntries > entries
	end
	local id, message = glue.net_begin(label, {
		confirmed = true,
		after = function(req)
			-- PoB's flags say why nothing came of it; a new build means the import worked
			if not req.new_build and not req.error and not compared() then
				local reason = not_imported(tab)
				req.error = reason
			end
		end,
	}, controls.importCodeGo.onClick)
	if id then
		return id
	end
	if compared() then
		return nil
	end
	-- nothing was fetched: a code imports at once, with Init run by the press
	if not compare and pob.build.importTab ~= tab then
		-- Init ran inside the press; a frame calculates the new build, as opening one does
		glue.frame()
		return nil
	end
	error(message or not_imported(tab), 0)
end

-- PoB's validator (`importCodeHandle`, ImportTab.lua:577) is a closure inside the ImportTab
-- constructor, so with no build open one throwaway ImportTab is built to run it. The constructor
-- runs with launches refused (as inside `Build:Init`, R4): it may validate a stored token.
local probe

local function validator_tab()
	if glue.build_loaded() then
		return pob.build.importTab
	end
	if not probe then
		__net_in_init(true)
		local ok, tab = pcall(new, "ImportTab", pob.build)
		__net_in_init(false)
		if not ok then
			error("couldn't start PoB's import box", 0)
		end
		probe = tab
	end
	return probe
end

-- Turns a `pob://` link into its https form, and checks what can be checked before anything is
-- cancelled or sent: the text is PoB's to judge (R16). Returns the text for the box.
local function normalise(url, mode)
	url = trim(url)
	if url:lower():match("^pob:") then
		local link = buildSites.ParseImportLinkFromURI(url)
		if not link then
			error("that pob:// link names no site PoB knows", 0)
		end
		url = link
	end
	if url == "" then
		error(INVALID, 0)
	end
	if mode == "this" and not (glue.build_loaded() and pob.build.dbFileName) then
		error("this build has no file yet; import into a new build", 0)
	end
	-- PoB's drop-down is disabled for a build with no file (ImportTab.lua:681)
	if mode == "compare" and not (glue.build_loaded() and pob.build.dbFileName) then
		error("PoB doesn't offer that here", 0)
	end
	return url
end

-- Raises what `glue.import_from_url` would raise for text PoB's box rejects, without acting.
-- The text goes into the box, as pasting it does.
function glue.import_check(url, mode)
	url = normalise(url, mode)
	local tab = validator_tab()
	checked = nil
	tab.controls.importCodeIn:SetText(url, true)
	if not tab.importCodeValid then
		error(not_imported(tab), 0)
	end
	checked = { tab = tab, url = url, buf = tab.controls.importCodeIn.buf, epoch = glue.net_epoch() }
end

-- Imports a link, a `pob://` link or a share code (R15, R16). Returns the request's id for a
-- link, or nil when a code was imported at once.
function glue.import_from_url(url, mode)
	url = normalise(url, mode)
	if glue.build_loaded() then
		return import_through_tab(url, mode)
	end
	-- no build: PoB's validator on a throwaway tab, then the download as Main:Init does it
	local tab = validator_tab()
	put_text(tab, url)
	if not tab.importCodeValid then
		error(not_imported(tab), 0)
	end
	if tab.importCodeSite and not tab.importCodeXML then
		-- the box unwrapped a redirect link: it holds the link to fetch
		return import_into_new_build(tab.controls.importCodeIn.buf, buildSites.websiteList[tab.importCodeSite])
	end
	if not tab.importCodeXML then
		error(INVALID, 0)
	end
	glue.enter_build(false, "Imported build", tab.importCodeXML)
	return nil
end

-- ---- Share ----

-- The Share button (ImportTab.lua:544): Generate, pick the site, upload. The link comes when the
-- request finishes; a failure is PoB's "Error creating link" message (R11).
function glue.share_build(site_id)
	local tab = import_tab()
	local controls = tab.controls
	-- PoB disables the button while an upload runs (it clears the box)
	if controls.generateCodeByLink.label == "Creating link..." then
		error("already sharing", 0)
	end
	local index
	for i, site in ipairs(controls.exportFrom.list) do
		if site.id == site_id then
			index = i
		end
	end
	if not index then
		error("no such site", 0)
	end
	controls.generateCode.onClick()
	controls.exportFrom:SelByValue(site_id, "id")
	-- the drop-down's own callback remembers the choice
	controls.exportFrom.selFunc(index, controls.exportFrom.list[index])
	if not controls.generateCodeByLink:IsShown() or not controls.generateCodeByLink:IsEnabled() then
		error("PoB doesn't offer that here", 0)
	end
	shared = nil
	local label = controls.exportFrom.list[index].label
	local id, message = glue.net_begin("Sharing on " .. label, {
		after = function(req)
			if not req.error then
				shared = { tab = tab, link = controls.generateCodeOut.buf }
			end
		end,
	}, controls.generateCodeByLink.onClick)
	if not id then
		error(message or "couldn't start the request", 0)
	end
	return id
end

-- ---- Account name import (R19) ----

local OFFERED = "PoB doesn't offer that here"

-- PoB's `charImportMode` as the view's stage
local STAGES = {
	GETACCOUNTNAME = "EnterAccount",
	DOWNLOADCHARLIST = "Fetching",
	SELECTCHAR = "SelectCharacter",
	IMPORTING = "Importing",
}

local REALMS = { PC = "Pc", XBOX = "Xbox", SONY = "Sony" }

-- PoB's sign-in, as its section shows it (R18, R21). The status is the label's own text: the
-- label function is called first, as Draw does, since it clears an expired login timer and a
-- passed rate limit. No token is read here, only whether there is one.
local function oauth_view(tab)
	local controls = tab.controls
	local label = controls.charImportStatusLabel.label
	local status = StripEscapes(type(label) == "function" and label() or label or "")
	status = status:gsub("^Character import status: ", "")
	local realm = controls.accountRealm:GetSelValue()
	local code = realm.realmCode
	local list = tab.characterList[code]
	local view = {
		signed_in = tab.isAuthorized() and true or false,
		signing_in = tab.oauthTimer ~= nil,
		realm = REALMS[realm.id] or "Pc",
		fetched = list ~= nil,
		leagues = {},
		characters = {},
		status = status,
		error = tab.oauthErrCode and StripEscapes(tostring(tab.oauthErrCode)) or nil,
		retry_at = tab.rateLimitEndTime,
		busy = tab.oauthLoading and true or false,
	}
	if tab.oauthTimer then
		view.login_deadline = tab.oauthTimer + 60
		local sign_in = glue.net_sign_in()
		view.auth_url = sign_in and sign_in.auth_url or nil
	end
	for _, league in ipairs(controls.charSelectLeague.list or {}) do
		view.leagues[#view.leagues + 1] = tostring(league)
	end
	-- every character of the realm, as `BuildCharacterList` filters and sorts them for "Any"
	for _, char in ipairs(list or {}) do
		if char.realm == code then
			view.characters[#view.characters + 1] = {
				name = tostring(char.name or "?"),
				class = tostring(char.class or "?"),
				level = math.floor(tonumber(char.level) or 0),
				league = tostring(char.league or "?"),
			}
		end
	end
	table.sort(view.characters, function(a, b)
		return a.name:lower() < b.name:lower()
	end)
	return view
end

-- The account import of the open build, as the tab shows it. The characters are all of the
-- account's, in the order PoB sorts a character list (by lowercase name), so a league can be
-- picked from them; `ImportPublicCharacter` selects the league itself.
function glue.account_view()
	local tab = import_tab()
	local controls = tab.controls
	local stage = STAGES[tab.charImportMode] or "EnterAccount"
	local view = {
		oauth = oauth_view(tab),
		public = {
			account = controls.siteAccountName.buf,
			realm = REALMS[controls.siteAccountRealm:GetSelValue().id] or "Pc",
			history = {},
			stage = stage,
			leagues = {},
			characters = {},
			status = StripEscapes(tab.charImportStatus or ""),
		},
	}
	local public = view.public
	for _, account in ipairs(controls.siteAccountHistory.list) do
		public.history[#public.history + 1] = account
	end
	if stage == "SelectCharacter" or stage == "Importing" then
		for _, entry in ipairs(controls.siteCharSelectLeague.list) do
			public.leagues[#public.leagues + 1] = entry.label
		end
		local code = controls.siteAccountRealm:GetSelValue().realmCode
		for _, char in ipairs(tab.lastCharList or {}) do
			if char.realm == code then
				public.characters[#public.characters + 1] = {
					name = tostring(char.name or "?"),
					class = tostring(char.class or "?"),
					level = math.floor(tonumber(char.level) or 0),
					league = tostring(char.league or "?"),
				}
			end
		end
		table.sort(public.characters, function(a, b)
			return a.name:lower() < b.name:lower()
		end)
	end
	return view
end

-- What PoB's own status says once a request of the account import ends, for the request's error.
-- A callback that raised leaves PoB's flags where its last line put them, and a status that
-- still describes the running request, so they are put right:
-- - the profile page's callback raises before it resets `DOWNLOADCHARLIST`, which PoB never
--   leaves (Start is gone and Close refused): back to the account box;
-- - the passive tree and item callbacks reset the stage first (`SELECTCHAR`) and raise later,
--   leaving "Retrieving ...": PoB's failure text replaces it.
local function status_after(tab, kind)
	return function(req)
		local status = tab.charImportStatus or ""
		if status:sub(1, #colorCodes.NEGATIVE) == colorCodes.NEGATIVE then
			-- PoB's own message says why
			req.error = StripEscapes(status)
		elseif kind == "list" and tab.charImportMode == "DOWNLOADCHARLIST" then
			local text = "Failed to retrieve character list."
			tab.charImportMode = "GETACCOUNTNAME"
			tab.charImportStatus = colorCodes.NEGATIVE .. text
			req.error = text
		elseif kind == "import" and req.error then
			local text = "Failed to retrieve character data, try again."
			tab.charImportStatus = colorCodes.NEGATIVE .. text
			req.error = text
		end
	end
end

-- The realm drop-down, the account name box and Start (ImportTab.lua:336-380)
function glue.list_public_characters(account, realm_id)
	local tab = import_tab()
	local controls = tab.controls
	if tab.charImportMode == "DOWNLOADCHARLIST" or tab.charImportMode == "IMPORTING" then
		error(OFFERED, 0)
	end
	local index
	for i, realm in ipairs(controls.siteAccountRealm.list) do
		if realm.id == realm_id then
			index = i
		end
	end
	if not index then
		error("no such realm", 0)
	end
	controls.siteAccountRealm:SetSel(index)
	if not glue.paste_into(controls.siteAccountName, account) then
		-- all control characters: the box would keep the account it had
		error("Missing discriminator e.g. #1234", 0)
	end
	local go = controls.siteAccountNameGo
	if not go:IsEnabled() then
		error("Missing discriminator e.g. " .. controls.siteAccountName.buf .. "#1234", 0)
	end
	if not go:IsShown() then
		error(OFFERED, 0)
	end
	local id, message = glue.net_begin("Fetching characters", { after = status_after(tab, "list") }, go.onClick)
	if not id then
		error(message or "couldn't start the request", 0)
	end
	return id
end

-- The league drop-down (nil is PoB's "All"), the character, the options and the part's button
-- (ImportTab.lua:411-466). The tree's confirm is pressed, because the UI asked first.
function glue.import_public_character(league, name, kind, a, b, c)
	local tab = import_tab()
	local controls = tab.controls
	if tab.charImportMode ~= "SELECTCHAR" then
		error(OFFERED, 0)
	end
	local index
	for i, entry in ipairs(controls.siteCharSelectLeague.list) do
		if entry.league == league and index == nil then
			index = i
		end
	end
	if not index then
		error("no such character", 0)
	end
	controls.siteCharSelectLeague:SetSel(index)
	local at
	for i, entry in ipairs(controls.siteCharSelect.list) do
		if entry.label == name and at == nil then
			at = i
		end
	end
	if not at then
		error("no such character", 0)
	end
	controls.siteCharSelect:SetSel(at)
	local button, label
	if kind == "tree" then
		option(controls.siteCharImportTreeClearJewels, a)
		button, label = controls.siteCharImportTree, "Importing passive tree"
	else
		option(controls.siteCharImportItemsClearItems, a)
		option(controls.siteCharImportItemsClearSkills, b)
		option(controls.siteCharImportItemsIgnoreWeaponSwap, c)
		button, label = controls.siteCharImportItems, "Importing items and skills"
	end
	if not button:IsShown() or not button:IsEnabled() then
		error(OFFERED, 0)
	end
	local id, message = glue.net_begin(label, { confirmed = true, after = status_after(tab, "import") }, button.onClick)
	if not id then
		error(message or "couldn't start the request", 0)
	end
	return id
end

-- PoB's Close button: back to the account box
function glue.close_public_characters()
	local tab = import_tab()
	local close = tab.controls.siteCharClose
	if tab.charImportMode == "DOWNLOADCHARLIST" or tab.charImportMode == "IMPORTING" then
		error(OFFERED, 0)
	end
	close.onClick()
end

-- ---- Sign in with Path of Exile (R1, R18, R21) ----
--
-- PoB's own buttons drive PoB's PoEAPI: Authorize starts LaunchServer.lua's listener, whose
-- `OpenURL` the glue keeps as the sign-in link (net.lua), and the redirect into it fetches the
-- token, which PoB saves in Settings.xml and nowhere else.

-- PoB's text for a body it can't read (PoEAPI.lua:48)
local MALFORMED = "Malformed response"
-- No `OpenURL` came: LaunchServer.lua didn't start, almost always because none of its three
-- ports was free. PoB's own error is nil (its `assert` on the `or` chain), shown as "nil".
local NO_PORT = "couldn't start the sign-in listener on localhost (ports 49082-49084)"

-- Puts right what a raising PoB callback leaves behind, and makes PoB's message the request's
-- error. The press cleared `oauthErrCode` (see `press`), so any message is this request's.
local function oauth_after(tab, sign_in)
	return function(req)
		local stuck
		if sign_in and not req.auth_url and not tab.isAuthorized() then
			-- the listener never started
			tab.oauthTimer = nil
			stuck = NO_PORT
		elseif sign_in and tab.oauthTimer then
			-- the token's callback raised on a body it couldn't read
			tab.oauthTimer = nil
			stuck = MALFORMED
		end
		if tab.oauthLoading then
			-- a list's callback raised on a body it couldn't read
			tab.oauthLoading = false
			stuck = stuck or MALFORMED
		end
		if stuck then
			tab.oauthErrCode = stuck
			req.error = stuck
		elseif tab.oauthErrCode then
			req.error = StripEscapes(tostring(tab.oauthErrCode))
		end
	end
end

-- Begins request `label` with a press of PoB's: the last press's error leaves the label first
-- (PoB clears it only on success), so the request's error is its own.
local function press(tab, label, opts, fn)
	tab.oauthErrCode = nil
	return glue.net_begin(label, opts, fn)
end

local function authorized_tab()
	local tab = import_tab()
	if not tab.isAuthorized() then
		error("sign in first", 0)
	end
	return tab
end

-- Selects realm `realm_id` in the drop-down, as a user's pick (its callback lists the leagues)
local function select_realm(controls, realm_id)
	local index
	for i, realm in ipairs(controls.accountRealm.list) do
		if realm.id == realm_id then
			index = i
		end
	end
	if not index then
		error("no such realm", 0)
	end
	controls.accountRealm:SetSel(index)
end

-- "Authorize with Path of Exile" (shown only when signed out). The link comes in the view.
function glue.sign_in()
	local tab = import_tab()
	local button = tab.controls.authenticateButton
	if not button:IsShown() or not button:IsEnabled() then
		error(OFFERED, 0)
	end
	-- PoB would start a second listener on the next port
	if glue.net_sign_in() then
		error("already signing in", 0)
	end
	local id, message = press(tab, "Signing in", { after = oauth_after(tab, true) }, button.onClick)
	if not id then
		error(message or "couldn't start the request", 0)
	end
	return id
end

-- "Logout from Path of Exile API": PoB forgets the tokens and saves its settings
function glue.sign_out()
	local button = import_tab().controls.logoutApiButton
	if not button:IsShown() or not button:IsEnabled() then
		error("sign in first", 0)
	end
	button.onClick()
end

-- The realm drop-down, and Fetch when PoB offers it (once per realm and tab). Returns the
-- request, or nil when nothing was fetched: the realm's list is there already, or PoB's rate
-- limiter answered at once (the view says so).
function glue.list_characters(realm_id)
	local tab = authorized_tab()
	-- a list fetch is out (the user's, or the sign-in's own): before its answer PoB's limiter
	-- knows no limits, would hold a second one until 2032 and keep that time for the tab's life
	if glue.net_sign_in() or glue.net_find("Fetching characters") or tab.oauthLoading then
		error("PoB is still loading your characters", 0)
	end
	local controls = tab.controls
	select_realm(controls, realm_id)
	local fetch = controls.accountRealmFetchButton
	if not fetch:IsEnabled() then
		return nil
	end
	local id = press(tab, "Fetching characters", { after = oauth_after(tab) }, fetch.onClick)
	return id
end

-- The realm, the league (nil is PoB's "Any"), the character, the part's options and its
-- button (ImportTab.lua:259-333). The tree's confirm is pressed, because the UI asked first.
-- Returns the request, or nil when PoB's rate limiter answered at once.
function glue.import_character(realm_id, league, name, kind, a, b, c)
	local tab = authorized_tab()
	local controls = tab.controls
	select_realm(controls, realm_id)
	local wanted = league or "Any"
	local index
	for i, entry in ipairs(controls.charSelectLeague.list or {}) do
		if entry == wanted and index == nil then
			index = i
		end
	end
	if not index then
		error("no such character", 0)
	end
	controls.charSelectLeague:SetSel(index)
	local at
	for i, entry in ipairs(controls.charSelect.list or {}) do
		if entry.label == name and at == nil then
			at = i
		end
	end
	if not at then
		error("no such character", 0)
	end
	controls.charSelect:SetSel(at)
	local button, label
	if kind == "tree" then
		option(controls.charImportTreeClearJewels, a)
		button, label = controls.charImportTree, "Importing passive tree"
	else
		option(controls.charImportItemsClearItems, a)
		option(controls.charImportItemsClearSkills, b)
		option(controls.charImportItemsIgnoreWeaponSwap, c)
		button, label = controls.charImportItems, "Importing items and skills"
	end
	if not button:IsShown() or not button:IsEnabled() then
		error(OFFERED, 0)
	end
	local id = press(tab, label, { confirmed = true, after = oauth_after(tab) }, button.onClick)
	return id
end
