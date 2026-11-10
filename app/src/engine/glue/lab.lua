-- Glue between the Rust host and Path of Building's headless calc engine.
--
-- Runs after bootstrap.lua, which exposes the build mode as `pob.build`.
-- Everything the Rust side needs is exposed through the global `lab` table.

lab = lab or {}

-- Scores every unallocated passive by how much `metric` (a key of the calc output,
-- e.g. "Life" or "CombinedDPS") would change if it were allocated. Mirrors PoB's own
-- node power: the node alone, and the node together with the path leading to it.
-- Uses PoB's what-if calculator, so the build itself is not modified.
-- `progress(done, total)` is called periodically.
function lab.node_power(metric, maxDistance, progress)
	local calc, base = pob.build.calcsTab:GetMiscCalculator()
	local baseValue = base[metric] or 0

	local candidates = {}
	for _, node in pairs(pob.build.spec.nodes) do
		local distance = node.path and #node.path or 0
		if not node.alloc and not node.ascendancyName and node.modKey ~= ""
			and node.type ~= "Mastery" and distance > 0 and distance <= maxDistance then
			table.insert(candidates, node)
		end
	end

	-- nodes with identical modifiers (e.g. "+10 to Strength") give identical results
	local singleCache = {}
	local results = {}
	for i, node in ipairs(candidates) do
		local single = singleCache[node.modKey]
		if not single then
			single = (calc({ addNodes = { [node] = true } })[metric] or 0) - baseValue
			singleCache[node.modKey] = single
		end

		local distance = #node.path
		local withPath = single
		if distance > 1 then
			local pathNodes = {}
			for _, pathNode in ipairs(node.path) do
				pathNodes[pathNode] = true
			end
			withPath = (calc({ addNodes = pathNodes })[metric] or 0) - baseValue
		end

		results[i] = {
			id = node.id,
			name = node.dn,
			kind = node.type,
			distance = distance,
			gain = single,
			path_gain = withPath,
			per_point = withPath / distance,
		}
		if i % 20 == 0 then
			progress(i, #candidates)
		end
	end
	return results
end
