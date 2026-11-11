base = "attack"
-- Craft a large cluster jewel: its skill and node count, two notables, then socket it in an
-- outer socket, which grows the tree.

-- The nearest unallocated outer jewel socket: the sockets of a large cluster jewel's size
local function outerSocket()
	local best
	for _, node in pairs(build.spec.nodes) do
		if node.type == "Socket" and node.expansionJewel and node.expansionJewel.size == 2
			and node.path and not node.alloc
			and (not best or node.pathDist < best.pathDist
				or (node.pathDist == best.pathDist and node.id < best.id)) then
			best = node
		end
	end
	assert(best, "no outer jewel socket is reachable")
	return best
end

-- The first entry of affix dropdown `slot` that adds a notable
local function notable(slot)
	local drop = build.itemsTab.controls["displayItemAffix" .. (slot + 1)]
	for i, entry in ipairs(drop.list) do
		if type(entry) == "table" and entry.label:find("Added Passive Skill is", 1, true) then return i - 1 end
	end
	error("no notable in slot " .. slot)
end

function recipe()
	local tab = build.itemsTab
	craftBase("Rare", "Gate Cluster", "Jewel: Cluster", "Large Cluster Jewel")
	local item = tab.displayItem
	assert(item and item.crafted and item.clusterJewel, "no crafted cluster jewel")
	assert(tab.controls.craftingSorting:IsShown() == false, "cluster jewels have no modifier sorting")

	act.cluster_skill(1)
	act.cluster_count(10)
	local dump = displayDump()
	assert(dump.cluster.count == 10 and dump.cluster.min == 8 and dump.cluster.max == 12, "the node count")
	assert(dump.cluster.skill.selected == 1, "the skill")
	assert(#dump.affixes > 0, "no affixes")
	assert(dump.crafting_sort == nil, "a cluster jewel is shown modifier sorting")
	checkpoint(true)

	act.affix(0, notable(0))
	act.affix(1, notable(1))
	local chosen = 0
	for _, affix in ipairs(displayDump().affixes) do
		if affix.list.selected > 0 then chosen = chosen + 1 end
	end
	assert(chosen == 2, "two notables expected, got " .. chosen)
	act.save()
	local id = tab.itemOrderList[#tab.itemOrderList]
	assert(tab.items[id].clusterJewel, "the saved item is not the cluster jewel")
	checkpoint(true)

	local before = layoutDigest()
	local socket = outerSocket()
	act.alloc(socket)
	act.equip("Jewel " .. socket.id, id)
	assert(layoutDigest() ~= before, "the tree did not grow")
	checkpoint(true)
end
