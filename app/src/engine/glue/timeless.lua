-- The timeless jewel search as a background job (R17).
--
-- PoB's Search press (`searchButton` of `TreeTab:FindTimelessJewel`) runs in a coroutine of its
-- own. A wrapper on `data.readLUT` (and on the Abyss jewels' `data.readAbyssJewelLUT`) yields
-- once the step's budget of `GetTime()` has passed, but only inside that coroutine: the tree
-- also reads the LUTs for socketed timeless jewels while it rebuilds between steps, and those
-- calls pass straight through.

glue = glue or {}

-- { co, popup, build, seeds, last_seed, total, start, budget }: nil while no search runs
local job

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

function glue.timeless_running()
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
		build = pob.build,
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
	if job.build ~= pob.build then
		job = nil
		return { done = true, percent = 100 }
	end
	job.budget = budget
	job.start = GetTime()
	local ok, err = coroutine.resume(job.co)
	if not ok then
		local popup = job.popup
		job = nil
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

-- Drops the running search. PoB replaces `searchResults` only once a search returns, so the
-- previous results stay (R17).
function glue.timeless_cancel()
	job = nil
end
