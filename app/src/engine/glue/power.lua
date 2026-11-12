-- The tree's node power (heat map): the settings, the calculation in slices and what it found.
-- Runs after bootstrap.lua, which exposes `pob.build`.

glue = glue or {}

-- The finished power report (TreeTab.lua:310-313): `{ stat, rows }`, or nil until a calculation ends.
local report

local function stat_by_label(label)
	for _, stat in ipairs(pob.build.treeTab.powerStatList) do
		if stat.label == label then
			return stat
		end
	end
	error("unknown heat map stat " .. tostring(label), 0)
end

-- The "Show Node Power" checkbox, the depth dropdown and the stat dropdown. `label` is a
-- `powerStatList` label; nil switches the heat map off. `maxDepth` nil is "All".
function glue.set_heatmap(label, maxDepth)
	local build = pob.build
	local tab = build.treeTab
	local stat = label and stat_by_label(label)
	-- nodePowerMaxDepthSelect's callback sets the depth (TreeTab.lua:222-227)
	build.calcsTab.nodePowerMaxDepth = maxDepth
	report = nil
	if stat then
		-- the stat dropdown's callback (TreeTab.lua:252-254) shows the heat map and starts a
		-- calculation (TreeTab.lua:1038)
		tab:SetPowerCalc(stat)
	else
		-- the checkbox, unticked (TreeTab.lua:190-199)
		tab.viewer.showHeatMap = false
		tab.controls.treeHeatMapStatSelect.shown = false
		tab.controls.powerReportList.shown = false
	end
end

-- Whether a calculation is waiting to start or running (CalcsTab.lua:469, 475).
function glue.power_pending()
	local calcs = pob.build.calcsTab
	return calcs.powerBuildFlag or calcs.powerBuilder ~= nil
end

-- One slice of CalcsTab:BuildPower (CalcsTab.lua:468-487), which resumes the coroutine itself
-- so that an error comes back instead of being dropped. Returns the progress the slice
-- reported (nil if none) and whether the calculation ended.
function glue.power_step()
	local build = pob.build
	local calcs = build.calcsTab
	local percent
	-- the one the tree tab installs shows toasts (TreeTab.lua:288)
	build.powerBuilderProgressCallback = function(value)
		percent = value
	end
	if calcs.powerBuildFlag then
		calcs.powerBuildFlag = false
		calcs.powerMax = nil
		calcs.powerBuilder = coroutine.create(calcs.PowerBuilder)
	end
	local finished = false
	local co = calcs.powerBuilder
	if co then
		local ok, err = coroutine.resume(co, calcs)
		if not ok then
			calcs.powerBuilder = nil
			error(tostring(err), 0)
		end
		if coroutine.status(co) == "dead" then
			calcs.powerBuilder = nil
			finished = true
			-- the completion callback (TreeTab.lua:310-313)
			local stat = calcs.powerStat or data.powerStatList[1]
			report = { stat = stat, rows = build.treeTab:BuildPowerReportList(stat) }
		end
	end
	return { percent = percent, finished = finished }
end

-- Every node PoB has a power for, and the maxima (CalcsTab.lua:468-731). Nodes that only have
-- a distance or mastery effects have no power of their own.
function glue.node_power()
	local build = pob.build
	local calcs = build.calcsTab
	local nodes = {}
	if build.treeTab.viewer.showHeatMap then
		for id, node in pairs(build.spec.nodes) do
			local power = node.power
			if power and (power.singleStat or power.pathPower or power.offence or power.defence) then
				nodes[#nodes + 1] = {
					id = id,
					single = power.singleStat or 0,
					path = power.pathPower,
					offence = power.offence,
					defence = power.defence,
				}
			end
		end
		table.sort(nodes, function(a, b) return a.id < b.id end)
	end
	local max = calcs.powerMax or {}
	return {
		stat = build.treeTab.viewer.showHeatMap and calcs.powerStat and calcs.powerStat.label or nil,
		nodes = nodes,
		max_single = max.singleStat or 0,
		max_offence = max.offence or 0,
		max_defence = max.defence or 0,
		-- a calculation that hasn't started yet has its flag set
		-- with the heat map off nothing is being calculated
		finished = not build.treeTab.viewer.showHeatMap
			or (calcs.powerBuilder == nil and not calcs.powerBuildFlag),
	}
end

-- The rows of BuildPowerReportList (TreeTab.lua:1054), with the text PoB shows. Cluster
-- notables have the distance "Cluster" (TreeTab.lua:1120), which becomes 0.
function glue.power_report()
	if not report then
		error("the power report isn't ready", 0)
	end
	local rows = {}
	for i, row in ipairs(report.rows) do
		rows[i] = {
			id = row.id,
			name = row.name,
			kind = row.type,
			power = row.power,
			power_text = row.powerStr,
			path_power = row.pathPower,
			path_power_text = row.pathPowerStr,
			allocated = row.allocated and true or false,
			path_dist = type(row.pathDist) == "number" and row.pathDist or 0,
		}
	end
	return { stat = report.stat.label, rows = rows }
end

-- ---- The power report's filter and masteries (NF-5) ----

-- The report as PoB's list shows it: the completion callback hands the rows to the list
-- (TreeTab.lua:310-313), whose ReList keeps the ones its filter and Show Masteries allow
-- (PowerReportListControl.lua:99-123). Positions (0-based) of `report.rows`, in the list's order.
local function listed_rows(rows)
	local list = pob.build.treeTab.controls.powerReportList
	local index = {}
	for i, row in ipairs(rows) do
		index[row] = i - 1
	end
	local listed = {}
	for _, row in ipairs(list.list or {}) do
		if index[row] then
			listed[#listed + 1] = index[row]
		end
	end
	return listed
end

local function report_list()
	return pob.build.treeTab.controls.powerReportList
end

local power_report = glue.power_report
function glue.power_report()
	local view = power_report()
	local list = report_list()
	if list.originalList ~= report.rows then
		list:SetReport(report.stat, report.rows)
	end
	view.listed = listed_rows(report.rows)
	view.filter = list.controls.filterSelect.selIndex - 1
	view.masteries = list.showMasteries and true or false
	return view
end

-- The filter dropdown (0-based) and the Show Masteries box, as the user sets them
function glue.set_power_report_filter(filter, masteries)
	local list = report_list()
	local select = list.controls.filterSelect
	if filter < 0 or filter >= #select.list then
		error("no such filter", 0)
	end
	if select.selIndex ~= filter + 1 then
		select:SetSel(filter + 1)
	end
	local check = list.controls.masteryCheck
	if (check.state and true or false) ~= masteries then
		check.state = masteries
		check.changeFunc(masteries)
	end
end
