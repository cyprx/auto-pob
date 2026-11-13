-- Helpers for the Compare gate tests. Runs in upstream PoB (HeadlessWrapper) with a build
-- loaded. Upstream presses PoB's own controls and reads what a whole frame draws (R4): nothing
-- here calls the Compare tab's draw functions by themselves.

-- A whole frame on the Compare tab's sub-view `mode`, with every `DrawString` recorded as
-- { y, text }. The build's own tab is put back afterwards.
local function compareFrame(mode, hook)
	local tab = build.compareTab
	local savedView, savedMode = build.viewMode, tab.compareViewMode
	build.viewMode = "COMPARE"
	tab.compareViewMode = mode
	local strings = {}
	local drawString = DrawString
	DrawString = function(x, y, align, height, font, text)
		strings[#strings + 1] = { y = y, height = height, text = text }
	end
	if hook then hook(true) end
	local ok, err = pcall(runCallback, "OnFrame")
	if hook then hook(false) end
	DrawString = drawString
	build.viewMode, tab.compareViewMode = savedView, savedMode
	assert(ok, err)
	assert(not launch.promptMsg, launch.promptMsg)
	return strings
end

-- The Summary as a frame draws it: the two build names of its header and its stat rows, each
-- the strings drawn at one `y`, between the "Difference" header and the power report.
function summaryFrame()
	local strings = compareFrame("SUMMARY")
	local first, last
	for i, s in ipairs(strings) do
		if s.text == "^7Difference" and not first then first = i end
		if s.text == "^7Compare Power Report" then last = i end
	end
	assert(first and last and last > first, "no Summary in the frame")
	assert(strings[first - 3].text == "^7Stat", "the Summary header moved")
	local rows, row, y = {}, nil, nil
	for i = first + 1, last - 1 do
		local s = strings[i]
		if not row or s.y ~= y then
			row = {}
			rows[#rows + 1] = row
			y = s.y
		end
		row[#row + 1] = s.text
	end
	return {
		headers = { strings[first - 2].text, strings[first - 1].text },
		rows = rows,
		positive = colorCodes.POSITIVE,
		negative = colorCodes.NEGATIVE,
	}
end

-- The overlay's node colours as a Tree frame computes them: every numeric colour this build's
-- viewer gets from `GetCompareNodeColor` while it draws, sorted by node.
function treeFrame()
	local viewer = build.treeTab.viewer
	local original = viewer.GetCompareNodeColor
	local seen = {}
	compareFrame("TREE", function(on)
		if on then
			viewer.GetCompareNodeColor = function(self, node, ...)
				local r, g, b = original(self, node, ...)
				if type(r) == "number" then seen[node.id] = { r, g, b } end
				return r, g, b
			end
		else
			viewer.GetCompareNodeColor = nil
		end
	end)
	local colours = {}
	for id, rgb in pairs(seen) do colours[#colours + 1] = { node = id, rgb = rgb } end
	table.sort(colours, function(a, b) return a.node < b.node end)
	return colours
end

-- A frame on `mode`, so PoB's draw fills the selectors, then `fn()` presses a control
function press(mode, fn)
	compareFrame(mode)
	local tab = build.compareTab
	local saved = tab.compareViewMode
	tab.compareViewMode = mode
	fn(tab.controls, tab)
	tab.compareViewMode = saved
	assert(#main.popups == 0, "a popup stayed open")
	compareFrame(mode)
end

-- The selectors of the top bar as the last frame left them
function selectors(names)
	local out = {}
	for i, name in ipairs(names) do
		local control = build.compareTab.controls[name]
		local labels = {}
		for j, item in ipairs(control.list or {}) do
			labels[j] = type(item) == "table" and item.label or item
		end
		out[i] = {
			name = name, shown = control:IsShown() and true or false,
			enabled = control:IsEnabled() and true or false, choices = labels,
			selected = math.max((control.selIndex or 1) - 1, 0),
		}
	end
	return out
end

-- The selectors of sub-view `mode` after a frame on it, read while it is shown
function selectorsIn(mode, names)
	compareFrame(mode)
	local tab = build.compareTab
	local saved = tab.compareViewMode
	tab.compareViewMode = mode
	local ok, out = pcall(selectors, names)
	tab.compareViewMode = saved
	assert(ok, out)
	return out
end

function entries()
	local tab = build.compareTab
	local labels = {}
	for i, entry in ipairs(tab.compareEntries) do labels[i] = entry.label end
	return { labels = labels, active = tab.activeCompareIndex - 1 }
end

-- The loaded build with a second skill set whose main group's gem grants two skills, so the
-- skill set and main skill selectors have something to change. Returns the build's XML.
function makeVariant()
	local skills = build.skillsTab
	skills:NewSkillSet(2)
	table.insert(skills.skillSetOrderList, 2)
	skills:SetActiveSkillSet(2)
	skills:PasteSocketGroup("Vaal Double Strike 20/0  1\r\nMelee Physical Damage 20/0  1\r\n")
	skills:SetActiveSkillSet(1)
	-- groups with a Stages box and a Mines box, for the main skill's count boxes
	skills:PasteSocketGroup("Scorching Ray 20/0  1\r\n")
	skills:PasteSocketGroup("Icicle Mine 20/0  1\r\n")
	build.buildFlag = true
	runCallback("OnFrame")
	return build:SaveDB("variant")
end

-- A share code of the XML, as PoB's Generate makes one
function shareCode(xml)
	return (common.base64.encode(Deflate(xml)):gsub("%+", "-"):gsub("/", "_"))
end

-- ---- Presses ----

-- What "Import from Folder" does with the file it read (CompareTab.lua:1563-1579)
function loadCompare(xml, label)
	assert(build.compareTab:ImportBuild(xml, label), "the import failed")
	compareFrame("SUMMARY")
end

-- The "Import..." popup: the code in its box, then its Import button
function loadCode(code)
	build.compareTab:OpenImportPopup()
	local controls = main.popups[1].controls
	controls.input:SetText(code, true)
	controls.go.onClick()
	assert(#main.popups == 0, "the import popup stayed open")
	compareFrame("SUMMARY")
end

-- "Compare with:" (1-based)
function pressSelect(index)
	press("SUMMARY", function(controls) controls.compareBuildSelect:SetSel(index) end)
end

-- Picks the entry, then Remove
function pressRemove(index)
	press("SUMMARY", function(controls)
		controls.compareBuildSelect:SetSel(index)
		controls.removeBtn.onClick()
	end)
end

-- A dropdown of the top bar (1-based)
function pressChoice(name, index)
	press("SUMMARY", function(controls)
		local control = controls[name]
		assert(control:IsShown() and control:IsEnabled(), name .. " is not offered")
		control:SetSel(index)
	end)
end

function pressCheck(name, on)
	press("TREE", function(controls)
		local control = controls[name]
		assert(control:IsShown() and control:IsEnabled(), name .. " is not offered")
		control.state = on
		control.changeFunc(on)
	end)
end

-- "Copy tree" or "Copy and use"
function pressCopy(andUse)
	press("TREE", function(controls)
		local button = andUse and controls.copySpecUseBtn or controls.copySpecBtn
		assert(button:IsShown() and button:IsEnabled(), "copy is not offered")
		button.onClick()
	end)
end

-- ---- Items, skills and config (R9) ----

-- The Items view's rows from a whole ITEMS frame: the slot list PoB's DrawItems built
-- (`AddAbyssSockets` fills it) and its jewel slots, with the items in them; plus every string
-- the frame drew, to check the rows are drawn in that order
function itemsFrame()
	local tab = build.compareTab
	local base, jewels
	local strings = compareFrame("ITEMS", function(on)
		if on then
			local addAbyss, jewelSlots = tab.AddAbyssSockets, tab.GetJewelComparisonSlots
			tab.AddAbyssSockets = function(self, comparison, dest, both)
				addAbyss(self, comparison, dest, both)
				base = dest
			end
			tab.GetJewelComparisonSlots = function(self, ...)
				jewels = jewelSlots(self, ...)
				return jewels
			end
		else
			tab.AddAbyssSockets, tab.GetJewelComparisonSlots = nil, nil
		end
	end)
	local entry = tab:GetActiveCompare()
	local pTab, cTab = build.itemsTab, entry.itemsTab
	-- what the frame's state says; the rest of each row is read from what it drew
	local rows = {}
	local function add(slot, label, pItem, cItem)
		rows[#rows + 1] = {
			slot = slot, label = label, ours = pItem and pItem.id or nil,
			differ = (pItem and pItem.raw) ~= (cItem and cItem.raw),
		}
	end
	for _, name in ipairs(base) do
		local pSlot, cSlot = pTab.slots[name], cTab.slots[name]
		add(name, name, pSlot and pTab.items[pSlot.selItemId], cSlot and cTab.items[cSlot.selItemId])
	end
	for _, jewel in ipairs(jewels) do
		add(jewel.cSlotName, jewel.label, jewel.pItem, jewel.cItem)
	end
	return { rows = rows, drawn = strings, warning = colorCodes.WARNING .. "  (tree missing allocated node)" }
end

-- PoB's "Expanded mode" box
function setItemsExpanded(on)
	local check = build.compareTab.controls.itemsExpandedCheck
	check.state = on
	check.changeFunc(on)
end

-- The cards a whole ITEMS frame draws in expanded mode: the strings of each DrawItemExpanded
-- call that draws (the measuring ones left out), in call order, so each slot's ours then theirs
function itemsExpandedFrame()
	local tab = build.compareTab
	local cards = {}
	compareFrame("ITEMS", function(on)
		if on then
			local draw = tab.DrawItemExpanded
			tab.DrawItemExpanded = function(self, item, x, y, width, other, measure)
				if measure then
					return draw(self, item, x, y, width, other, measure)
				end
				local lines = {}
				local outer = DrawString
				DrawString = function(_, _, _, _, _, text)
					lines[#lines + 1] = text
				end
				local ok, result = pcall(draw, self, item, x, y, width, other, measure)
				DrawString = outer
				assert(ok, result)
				cards[#cards + 1] = lines
				return result
			end
		else
			tab.DrawItemExpanded = nil
		end
	end)
	return cards
end

-- What DrawSkills draws in a whole SKILLS frame: { x, y, height, text }, the build names first
function skillsFrame()
	local tab = build.compareTab
	local strings = {}
	compareFrame("SKILLS", function(on)
		if on then
			local drawSkills = tab.DrawSkills
			tab.DrawSkills = function(self, ...)
				local outer = DrawString
				DrawString = function(x, y, align, height, font, text)
					strings[#strings + 1] = { x = x, y = y, height = height, text = text }
				end
				local ok, err = pcall(drawSkills, self, ...)
				DrawString = outer
				assert(ok, err)
			end
		else
			tab.DrawSkills = nil
		end
	end)
	return { strings = strings, added = colorCodes.POSITIVE .. "+ ", missing = colorCodes.NEGATIVE .. "- " }
end

-- The rows a whole CONFIG frame marks as differing (LayoutConfigView's section layout), with
-- PoB's value texts
function configFrame()
	compareFrame("CONFIG")
	local tab = build.compareTab
	local entry = tab:GetActiveCompare()
	local pInput, cInput = build.configTab.input, entry.configTab.input
	local rows = {}
	for _, section in ipairs(tab.configSectionLayout) do
		for _, row in ipairs(section.rows) do
			local varData = row.ctrlInfo.varData
			rows[#rows + 1] = {
				var = varData.var, label = varData.label or varData.var,
				ours = tab:FormatConfigValue(varData, pInput[varData.var]),
				theirs = tab:FormatConfigValue(varData, cInput[varData.var]),
				common = not row.isDiff,
			}
		end
	end
	return rows
end

-- "Show All Configurations" / "Hide Ineligible Configurations"
function pressConfigToggle()
	press("CONFIG", function(controls)
		assert(controls.configToggleBtn:IsShown(), "the toggle is not offered")
		controls.configToggleBtn.onClick()
	end)
end

-- A Tree view version dropdown (1-based), then the convert popup's button
function pressVersion(name, index, keepCopy)
	press("TREE", function(controls)
		local control = controls[name]
		assert(control:IsShown() and control:IsEnabled(), name .. " is not offered")
		control:SetSel(index)
		assert(#main.popups == 1, "no convert popup")
		main.popups[1].controls[keepCopy and "convertCopy" or "convert"].onClick()
	end)
end

-- Both builds' trees: this build's specs (title, version) and active one, the comparison's
-- version, and each active spec's allocated nodes
function specState()
	local entry = build.compareTab:GetActiveCompare()
	local function nodes(spec)
		local out = {}
		for id in pairs(spec.allocNodes) do out[#out + 1] = id end
		table.sort(out)
		return out
	end
	local specs = {}
	for i, spec in ipairs(build.treeTab.specList) do
		specs[i] = { spec.title or "Default", spec.treeVersion }
	end
	return {
		specs = specs, active = build.treeTab.activeSpec, ours = nodes(build.spec),
		theirs_version = entry.spec.treeVersion, theirs = nodes(entry.spec),
	}
end

local function dumpTooltip(tt)
	local out = { influences = {}, lines = {} }
	if tt.tooltipHeader then out.header = tostring(tt.tooltipHeader) end
	if tt.influenceHeader1 then out.influences[#out.influences + 1] = tt.influenceHeader1 end
	if tt.influenceHeader2 then out.influences[#out.influences + 1] = tt.influenceHeader2 end
	out.foil = tt.foilType or nil
	if type(tt.color) == "string" then out.colour = tt.color end
	for i, line in ipairs(tt.lines) do
		if line.text == nil then
			out.lines[i] = { separator = line.size }
		else
			out.lines[i] = { size = line.size, text = line.text, font = line.font, center = line.center or false }
			if type(line.background) == "string" then out.lines[i].background = line.background end
		end
	end
	return out
end

-- What hovering the comparison's item in a base slot shows (CompareTab.lua:3962-3965)
function theirTooltip(slot)
	local entry = build.compareTab:GetActiveCompare()
	local cTab = entry.itemsTab
	local item = cTab.items[cTab.slots[slot].selItemId]
	local tt = new("Tooltip")
	local width = GetScreenSize() - 312
	cTab:AddItemTooltip(tt, item, nil, nil, math.min(600, math.max(260, width - 24)))
	return dumpTooltip(tt)
end

-- The Copy / Equip buttons' call (CompareTab.lua:3936-3941)
function copyItem(slot, andUse)
	local tab = build.compareTab
	tab:CopyCompareItemToPrimary(slot, tab:GetActiveCompare(), andUse)
	compareFrame("ITEMS")
end

-- This build's items as raw texts in list order, and what the slot holds
function itemState(slot)
	local t = build.itemsTab
	local raws = {}
	for i, id in ipairs(t.itemOrderList) do raws[i] = t.items[id]:BuildRaw() end
	local sel = t.items[t.slots[slot].selItemId]
	return { raws = raws, slot = sel and sel:BuildRaw() or nil, set = t.activeItemSetId }
end

function configInput()
	local out = {}
	for k, v in pairs(build.configTab.input) do out[k] = tostring(v) end
	return out
end

-- An edit box of the top bar: typed as PoB's EditControl takes it
function pressText(name, text)
	press("SUMMARY", function(controls)
		local control = controls[name]
		assert(control:IsShown() and control:IsEnabled(), name .. " is not offered")
		control:SetText(text, true)
	end)
end

-- A dropdown of the Items view (1-based)
function pressItemsChoice(name, index)
	press("ITEMS", function(controls)
		local control = controls[name]
		assert(control:IsShown() and control:IsEnabled(), name .. " is not offered")
		control:SetSel(index)
	end)
end

function pressCopyConfig()
	press("CONFIG", function(controls)
		assert(controls.copyConfigBtn:IsShown(), "copy config is not offered")
		controls.copyConfigBtn.onClick()
	end)
end

-- The loaded build with an amulet granting "+1 Ring Slot", a ring in Ring 3, and a jewel in a
-- socket its tree doesn't allocate. Returns the build's XML.
function makeRing3Variant()
	local tab = build.itemsTab
	local function add(text, slot)
		local item = new("Item", text)
		tab:AddItem(item, true)
		if slot then
			tab.slots[slot]:SetSelItemId(item.id)
		end
		return item
	end
	add([[Rarity: RARE
Gate Clasp
Onyx Amulet
Implicits: 0
+1 Ring Slot
+20 to maximum Life]], "Amulet")
	add([[Rarity: RARE
Gate Band
Coral Ring
Implicits: 0
+33 to maximum Life]], "Ring 3")
	-- named as the gear setup's Ring 1, with another roll: PoB's "(match)", different texts
	add([[Rarity: RARE
Doom Loop
Iron Ring
Implicits: 0
+31 to maximum Life]], "Ring 1")
	local socket
	for id, node in pairs(build.spec.nodes) do
		if node.type == "Socket" and not node.alloc and (not socket or id < socket) then socket = id end
	end
	add([[Rarity: MAGIC
Crimson Jewel
Implicits: 0
+12 to Strength]], "Jewel " .. socket)
	tab:PopulateSlots()
	build.buildFlag = true
	runCallback("OnFrame")
	assert(build.calcsTab.mainEnv.modDB:Flag(nil, "AdditionalRingSlot"), "no third ring slot")
	return build:SaveDB("ring3")
end

-- Items undo, as the tab's Undo
function itemsUndo()
	build.itemsTab:Undo()
	runCallback("OnFrame")
end

-- The Skills tab's Delete All button and its confirmation (SkillListControl.lua:27-41)
function deleteAllGroups()
	build.skillsTab.controls.groupList.controls.deleteAll.onClick()
	main.popups[1].controls.confirm.onClick()
	runCallback("OnFrame")
end
