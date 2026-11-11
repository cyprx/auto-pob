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
			glue.enter_build(false, "Imported build", xml, false, importLink)
		end)
	end)
	if not id then
		error(message or "couldn't start the request", 0)
	end
	return id
end

-- With a build open: the box takes the text, the mode drop-down the target, Go does the rest.
local function import_through_tab(url, mode)
	local tab = import_tab()
	local controls = tab.controls
	if mode == "this" and not pob.build.dbFileName then
		error("this build has no file yet; import into a new build", 0)
	end
	controls.importCodeIn:SetText(url, true)
	if not tab.importCodeValid then
		error(not_imported(tab), 0)
	end
	if mode == "this" then
		if not controls.importCodeMode:IsEnabled() then
			error("PoB doesn't offer that here", 0)
		end
		controls.importCodeMode:SetSel(1)
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
	local id, message = glue.net_begin(label, {
		confirmed = true,
		after = function(req)
			-- PoB's flags say why nothing came of it; a new build means the import worked
			if not req.new_build and not req.error then
				local reason = not_imported(tab)
				req.error = reason
			end
		end,
	}, controls.importCodeGo.onClick)
	if id then
		return id
	end
	-- nothing was fetched: a code imports at once, with Init run by the press
	if pob.build.importTab ~= tab then
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
	return url
end

-- Raises what `glue.import_from_url` would raise for text PoB's box rejects, without acting.
-- The text goes into the box, as pasting it does.
function glue.import_check(url, mode)
	url = normalise(url, mode)
	local tab = validator_tab()
	tab.controls.importCodeIn:SetText(url, true)
	if not tab.importCodeValid then
		error(not_imported(tab), 0)
	end
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
	tab.controls.importCodeIn:SetText(url, true)
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
