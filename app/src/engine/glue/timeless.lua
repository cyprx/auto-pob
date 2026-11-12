-- The timeless jewel search as a background job (R17).
--
-- PoB's Search press (`searchButton` of `TreeTab:FindTimelessJewel`) runs in a coroutine of its
-- own. A wrapper on `data.readLUT` (and on the Abyss jewels' `data.readAbyssJewelLUT`) yields
-- once the step's budget of `GetTime()` has passed, but only inside that coroutine: the tree
-- also reads the LUTs for socketed timeless jewels while it rebuilds between steps, and those
-- calls pass straight through.

glue = glue or {}

-- { co, popup, gen, seeds, last_seed, total, start, budget, saved }: nil while no search runs
local job

-- The previous search as the window shows it: PoB's "All Sockets" branch wipes the results
-- before it searches (TreeTab.lua:2795-2800), so a search that doesn't finish puts these back
-- (R17). The list control holds the same tables, so they are refilled in place.
local function save(popup, timelessData)
	local c = popup.controls
	return {
		results = copyTable(timelessData.searchResults, true),
		shared = copyTable(timelessData.sharedResults, true),
		trade = {
			enabled = c.searchTradeButton.enabled,
			lastSearch = c.searchTradeButton.lastSearch,
			label = c.searchTradeButton.label,
		},
		selIndex = c.searchResults.selIndex,
		highlightIndex = c.searchResults.highlightIndex,
	}
end

local function refill(target, from)
	wipeTable(target)
	for k, v in pairs(from) do
		target[k] = v
	end
end

local function restore(stopped)
	local saved = stopped.saved
	local timelessData = stopped.timelessData
	refill(timelessData.searchResults, saved.results)
	refill(timelessData.sharedResults, saved.shared)
	local c = stopped.popup.controls
	c.searchTradeButton.enabled = saved.trade.enabled
	c.searchTradeButton.lastSearch = saved.trade.lastSearch
	c.searchTradeButton.label = saved.trade.label
	c.searchResults.selIndex = saved.selIndex
	c.searchResults.highlightIndex = saved.highlightIndex
end

local function wrap(name)
	local read = data[name]
	data[name] = function(seed, ...)
		if job and coroutine.running() == job.co then
			-- progress counts the seeds, which every socket searched goes through in turn
			if seed ~= job.last_seed then
				job.last_seed = seed
				job.seeds = job.seeds + 1
			end
			if GetTime() - job.start >= job.budget then
				coroutine.yield()
			end
		end
		return read(seed, ...)
	end
end

local wrapped = false

local function install()
	if not wrapped then
		wrap("readLUT")
		wrap("readAbyssJewelLUT")
		wrapped = true
	end
end

-- Drops the job without its results: a search that doesn't finish leaves the previous ones
local function stop()
	local stopped = job
	job = nil
	if stopped then
		restore(stopped)
	end
end

function glue.timeless_running()
	-- a build that PoB initialised again (a failed open falls back to a new one) ends it
	if job and job.gen ~= glue.build_generation() then
		stop()
	end
	return job ~= nil
end

-- Starts PoB's Search press of `popup` as the job. Returns at once.
function glue.timeless_start(popup, button)
	if job then
		error("a timeless search is already running", 0)
	end
	install()
	local timelessData = pob.build.timelessData
	local id = timelessData.jewelType.id
	-- "All Sockets" searches every socket of the list after itself (TreeTab.lua:2805-2806)
	local sockets = timelessData.jewelSocket.id == -1 and (#popup.controls.socketSelect.list - 1) or 1
	local seeds = data.timelessJewelSeedMax[id] - data.timelessJewelSeedMin[id] + 1
	job = {
		co = coroutine.create(function()
			button.onClick()
		end),
		popup = popup,
		gen = glue.build_generation(),
		timelessData = timelessData,
		saved = save(popup, timelessData),
		seeds = 0,
		total = math.max(1, sockets * seeds),
		budget = 0,
		start = 0,
	}
end

-- How far the search is, 0..100; approximate: the seeds read over every socket's seeds
function glue.timeless_progress()
	if not job then
		return 100
	end
	return math.min(99, math.floor(job.seeds * 100 / job.total))
end

-- Runs the search for about `budget` ms. Returns `{ done, percent }`. A Lua error ends the job,
-- with PoB's message in the window's `msg` label, and is raised.
function glue.timeless_step(budget)
	if not job then
		return { done = true, percent = 100 }
	end
	if not glue.timeless_running() then
		return { done = true, percent = 100 }
	end
	job.budget = budget
	job.start = GetTime()
	local ok, err = coroutine.resume(job.co)
	if not ok then
		local popup = job.popup
		stop()
		err = tostring(err)
		popup.controls.msg.label = "^1" .. err
		error(err, 0)
	end
	if coroutine.status(job.co) == "dead" then
		job = nil
		return { done = true, percent = 100 }
	end
	return { done = false, percent = glue.timeless_progress() }
end

-- Drops the running search; the previous results stay, as PoB never shows a search that
-- didn't return (R17).
function glue.timeless_cancel()
	stop()
end
