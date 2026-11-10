base = "new"
-- `link_url` is the attack tree's link, exported by a second upstream build (link_from).
link_from = "attack"
function recipe()
	act.import(link_url, "Imported")
	checkpoint()
end
