$global:__NoctermPrompt = $function:prompt
function global:prompt {
  $p = [System.Uri]::new((Get-Location).Path).AbsoluteUri
  [Console]::Write("`e]7;$p`a`e]133;A`a")
  if ($global:__NoctermPrompt) { & $global:__NoctermPrompt } else { "PS $pwd> " }
  [Console]::Write("`e]133;B`a")
}
if (Get-Module PSReadLine) {
  Set-PSReadLineKeyHandler -Key Enter -ScriptBlock {
    [Console]::Write("`e]133;C`a")
    [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
  }
}
