base = "attack"
function recipe()
	-- set 1 has no title ("Default"); the new sets get the first free ids, 2 and 3
	act.new_set("Mapping")
	act.copy_set(1, "Bossing")
	checkpoint()
	act.select_set(3)
	act.paste("Label: Bossing aura\r\nHatred 20/0  1\r\n")
	act.rename_set(2, "Clear")
	checkpoint()
	-- the active set goes: PoB picks the one before it
	act.delete_set(3)
	checkpoint()
	act.undo()
	checkpoint()
end
