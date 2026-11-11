-- Helpers for the network recipes. Runs in upstream PoB after tree_lib.lua, with the subscript
-- natives installed and their scope open. Every `act.*` presses PoB's own controls and leaves
-- the popup open for the pump to deliver into, as PoB does. Nothing here may call our glue.

local function frame()
	runCallback("OnFrame")
end

-- The Import button on a PoEURL link (TreeTab.lua:741-771)
function act.tree_import(url, title)
	build.treeTab:OpenImportPopup()
	local controls = main.popups[1].controls
	controls.name:SetText(title, true)
	controls.edit:SetText(url, true)
	controls.import.onClick()
	frame()
end

-- The Export popup's "Shrink with PoEURL" button (TreeTab.lua:813-831)
function act.tree_shrink()
	build.treeTab:OpenExportPopup()
	main.popups[1].controls.shrink.onClick()
	frame()
end

-- What the newest popup tells the user: the import popup's message, or a message popup's text
function popupMessage()
	local popup = main.popups[1]
	if not popup then return nil end
	local controls = popup.controls
	if controls.msg then return StripEscapes(controls.msg.label) end
	if controls[1] then return StripEscapes(controls[1].label) end
end

-- The Export popup's link box after the shrink
function shrunkLink()
	for _, popup in ipairs(main.popups) do
		if popup.controls.shrink then return popup.controls.edit.buf end
	end
end

-- Closes what the pump left open
function closePopups()
	while #main.popups > 0 do main:ClosePopup() end
end

-- The tree state, as `checkpoint()` records it; `tree_lib.lua`'s own
function treeDump()
	checkpoint()
	return actions[#actions].check
end

function treeCanUndo()
	return build.spec.undo[2] ~= nil
end

-- ---- The Import/Export tab (ImportTab.lua:504-728) ----

-- Pastes into the import box; its validator runs
function act.code_set(text)
	build.importTab.controls.importCodeIn:SetText(text, true)
end

-- The box, the target drop-down and Go, as a user does. A confirm popup stays open until
-- `act.confirm()`; a download is delivered by the pump.
function act.code_import(text, mode)
	local controls = build.importTab.controls
	controls.importCodeIn:SetText(text, true)
	controls.importCodeMode:SetSel(mode == "this" and 1 or 2)
	controls.importCodeGo.onClick()
	frame()
end

-- The confirm button of the newest popup
function act.confirm()
	main.popups[1].controls.confirm.onClick()
	frame()
end

-- Generate, pick the site in the drop-down, press Share
function act.share(site)
	local controls = build.importTab.controls
	controls.generateCode.onClick()
	for index, entry in ipairs(controls.exportFrom.list) do
		if entry.id == site then controls.exportFrom:SetSel(index) end
	end
	controls.generateCodeByLink.onClick()
	frame()
end

-- What the tab shows
function importDump()
	local tab = build.importTab
	local controls = tab.controls
	local sites = {}
	for _, entry in ipairs(controls.exportFrom.list) do sites[#sites + 1] = entry.id end
	table.sort(sites)
	local site = tab.importCodeSite and buildSites.websiteList[tab.importCodeSite].id
	return {
		detail = StripEscapes(tab.importCodeDetail or ""),
		valid = tab.importCodeValid and true or false,
		site = site,
		text = controls.importCodeIn.buf,
		link = controls.generateCodeOut.buf,
		sites = sites,
		fetching = tab.importCodeFetching and true or false,
		share_label = controls.generateCodeByLink.label,
		last_site = tab.exportWebsiteSelected,
		unsaved = build.unsaved and true or false,
		import_link = build.importLink,
	}
end

-- The sites PoB knows, as data
function siteList()
	local list = {}
	for _, site in ipairs(buildSites.websiteList) do
		list[#list + 1] = {
			id = site.id, label = site.label, download = site.downloadURL,
			post_url = site.postUrl, post_fields = site.postFields, code_out = site.codeOut,
		}
	end
	return list
end

-- A protocol link as Main:Init handles `arg[1]` (Main.lua:68-77): fetched with no ImportTab,
-- then opened as a new build with the callback's link
function act.arg_import(uri)
	buildSites.DownloadBuild(uri, nil, function(isSuccess, data, importLink)
		if not isSuccess then
			main:SetMode("BUILD", false, data)
		else
			local xmlText = Inflate(common.base64.decode(data:gsub("-","+"):gsub("_","/")))
			main:SetMode("BUILD", false, "Imported Build", xmlText, false, importLink)
		end
	end)
end

-- ---- Import by account name (ImportTab.lua:336-472, 813-1018) ----

-- Realm drop-down, the box, Start: a user's steps. The button must be enabled, as PoB's is.
function act.public_list(account, realm)
	local controls = build.importTab.controls
	controls.siteAccountRealm:SelByValue(realm, "id")
	controls.siteAccountName:SetText(account, true)
	if not controls.siteAccountNameGo:IsEnabled() then
		return "Missing discriminator e.g. " .. controls.siteAccountName.buf .. "#1234"
	end
	controls.siteAccountNameGo.onClick()
	frame()
end

-- The league drop-down (nil is "All"), the character, the options as clicks and the button
function act.public_import(league, name, part, a, b, c)
	local controls = build.importTab.controls
	local leagues = controls.siteCharSelectLeague
	for index, entry in ipairs(leagues.list) do
		if entry.league == league then leagues:SetSel(index) break end
	end
	for index, entry in ipairs(controls.siteCharSelect.list) do
		if entry.label == name then controls.siteCharSelect:SetSel(index) break end
	end
	local function click(box, on)
		if box.state ~= on then
			box.state = not box.state
		end
	end
	if part == "tree" then
		click(controls.siteCharImportTreeClearJewels, a)
		controls.siteCharImportTree.onClick()
	else
		click(controls.siteCharImportItemsClearItems, a)
		click(controls.siteCharImportItemsClearSkills, b)
		click(controls.siteCharImportItemsIgnoreWeaponSwap, c)
		controls.siteCharImportItems.onClick()
	end
	frame()
end

function act.public_close()
	build.importTab.controls.siteCharClose.onClick()
end

function act.frame()
	frame()
end

-- What the account name import shows, in PoB's own terms
function accountDump()
	local tab = build.importTab
	local controls = tab.controls
	local out = {
		account = controls.siteAccountName.buf,
		realm = controls.siteAccountRealm:GetSelValue().id,
		stage = tab.charImportMode,
		status = StripEscapes(tab.charImportStatus),
		history = {}, leagues = {}, characters = {},
		prompt = main.promptMsg ~= nil or launch.promptMsg ~= nil,
		popups = #main.popups,
	}
	for _, account in ipairs(controls.siteAccountHistory.list) do out.history[#out.history + 1] = account end
	for _, entry in ipairs(controls.siteCharSelectLeague.list) do out.leagues[#out.leagues + 1] = entry.label end
	-- PoB's own list for the "All" entry: its realm filter and its sort
	local all
	for index, entry in ipairs(controls.siteCharSelectLeague.list) do
		if entry.league == nil then all = index end
	end
	if all then
		controls.siteCharSelectLeague:SetSel(all)
		for _, entry in ipairs(controls.siteCharSelect.list) do
			local char = entry.char
			out.characters[#out.characters + 1] = { name = char.name, class = char.class, level = char.level, league = char.league }
		end
	end
	return out
end

-- The sidebar-independent facts a character import must have produced
function importedFacts()
	local equipped = 0
	for _, slot in pairs(build.itemsTab.slots) do
		if slot.selItemId ~= 0 then equipped = equipped + 1 end
	end
	return {
		groups = #build.skillsTab.socketGroupList,
		nodes = build.spec:CountAllocNodes(),
		equipped = equipped,
		ascend = build.spec.curAscendClassId,
		level = build.characterLevel,
		build_flag = build.buildFlag and true or false,
		mod_flag = build.modFlag and true or false,
		spec_mod = build.spec.modFlag and true or false,
		items_mod = build.itemsTab.modFlag and true or false,
		config_mod = build.configTab.modFlag and true or false,
	}
end

-- ---- Import from a signed-in account (ImportTab.lua:24-334) ----
-- Upstream never runs PoEAPI: its devMode would write poe_api_response.json into the PoB folder.
-- PoB's own functions take the canned JSON instead.

local dkjson = require("dkjson")

-- What the section shows for `realm`'s list `json` (the character list endpoint's body): the
-- realm drop-down's own callback lists the leagues, `BuildCharacterList` the characters of
-- "Any" (its realm filter and its sort)
function oauthDump(json, realm)
	local tab = build.importTab
	local controls = tab.controls
	local list = dkjson.decode(json).characters
	controls.accountRealm:SelByValue(realm, "realmCode")
	tab.characterList[realm] = list
	controls.accountRealm.selFunc()
	local out = { leagues = {}, characters = {} }
	for _, league in ipairs(controls.charSelectLeague.list) do out.leagues[#out.leagues + 1] = league end
	local scratch = { list = {} }
	tab:BuildCharacterList(realm, nil, list, scratch)
	for _, entry in ipairs(scratch.list) do
		local char = entry.char
		out.characters[#out.characters + 1] = { name = char.name, class = char.class, level = char.level, league = char.league }
	end
	return out
end

-- A part of the character endpoint's body `json`, imported as the account import's buttons do
-- with their default boxes
function oauthImport(json, part)
	local character = dkjson.decode(json).character
	if part == "tree" then
		build.importTab:ImportPassiveTreeAndJewels(character, true)
	else
		build.importTab:ImportItemsAndSkills(character, true, true, false)
	end
	frame()
end
