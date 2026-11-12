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
		strings[#strings + 1] = { y = y, text = text }
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
