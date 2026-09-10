Set fso = CreateObject("Scripting.FileSystemObject")
Set shell = CreateObject("WScript.Shell")

scriptDir = fso.GetParentFolderName(WScript.ScriptFullName)
watchdog = fso.BuildPath(scriptDir, "codex-discord-watchdog.ps1")
command = "powershell.exe -WindowStyle Hidden -NoProfile -ExecutionPolicy Bypass -File " & Quote(watchdog)

shell.Run command, 0, False

Function Quote(value)
    Quote = """" & Replace(value, """", """""") & """"
End Function
